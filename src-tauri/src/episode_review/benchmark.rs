//! Episode review benchmark (test-data/episode-review-benchmark.json): finished episodes
//! with the cards the app should and should not show. Most cases are near misses that must
//! never become a completion card: drafts, pre-join screens, errors, time alone, another
//! person's result, the wrong recipient or meeting, old history and injected text.
//! The offline test checks the data and the scoring; the live test asks the real model.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Case {
    id: String,
    goals: Vec<CaseGoal>,
    windows: Vec<CaseWindow>,
    expect: Expect,
}
#[derive(Deserialize)]
struct CaseGoal {
    id: i64,
    title: String,
    #[serde(default)]
    steps: Vec<CaseStep>,
    #[serde(default)]
    meaning: Option<String>,
    #[serde(default)]
    targets: Vec<String>,
}
#[derive(Deserialize)]
struct CaseStep {
    id: String,
    text: String,
}
#[derive(Deserialize)]
struct CaseWindow {
    app: String,
    title: String,
    #[serde(default)]
    site: Option<String>,
    seconds: i64,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    goal: Option<i64>,
    #[serde(default)]
    text: Option<String>,
}
#[derive(Deserialize)]
struct Expect {
    /// "goal" or "goal:step" for each card that should be offered; empty for near misses.
    complete: Vec<String>,
    new_goal: bool,
    /// Window index to goal for unmatched windows that should be attributed.
    #[serde(default)]
    attribute: BTreeMap<String, i64>,
}

/// What the app would do with an answer: the same thresholds as `finish`.
#[derive(Debug, PartialEq)]
struct Effect {
    complete: BTreeSet<String>,
    new_goal: bool,
    attribute: BTreeMap<String, i64>,
}
fn effect(job: &Job, answer: &Answer) -> Effect {
    Effect {
        complete: answer
            .outcomes
            .iter()
            .filter(|o| {
                o.status == "achieved" && o.confidence >= PROPOSE_AT && !o.evidence.is_empty()
            })
            .map(|o| match &o.step_id {
                Some(step) => format!("{}:{step}", o.goal_id),
                None => o.goal_id.to_string(),
            })
            .collect(),
        new_goal: answer.new_goal.as_ref().is_some_and(|g| {
            g.confidence >= NEW_GOAL_AT && job.unassigned_seconds >= NEW_GOAL_SECONDS
        }),
        attribute: answer
            .windows
            .iter()
            .filter(|w| job.windows[w.index].goal_id.is_none() && w.confidence >= ATTRIBUTE_AT)
            .filter_map(|w| w.goal_id.map(|g| (w.index.to_string(), g)))
            .collect(),
    }
}
/// Whether the effect matches; attribution is only checked for the windows a case names.
fn passes(case: &Case, got: &Effect) -> bool {
    got.complete
        == case
            .expect
            .complete
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
        && got.new_goal == case.expect.new_goal
        && case
            .expect
            .attribute
            .iter()
            .all(|(i, g)| got.attribute.get(i) == Some(g))
}
/// The request `claim` would build for the case, in the same shape.
fn job(case: &Case) -> Job {
    let windows: Vec<EpisodeEvidence> = case
        .windows
        .iter()
        .enumerate()
        .map(|(i, w)| EpisodeEvidence {
            period_id: 1,
            at: format!("2026-11-10T09:{:02}:00Z", i * 5),
            last_at: format!("2026-11-10T09:{:02}:00Z", i * 5 + 4),
            context_key: format!("k{i}"),
            app: w.app.clone(),
            site: w.site.clone(),
            excerpt_ref: None,
            result_signal: w.result.clone(),
            goal_id: w.goal,
            step_id: None,
            provenance: "window_metadata".into(),
            consent_revision: 0,
            seconds: w.seconds,
            title: w.title.clone(),
        })
        .collect();
    let unassigned = case
        .windows
        .iter()
        .filter(|w| w.goal.is_none())
        .map(|w| w.seconds)
        .sum();
    let context = json!({
        "episode": {"started_at": "2026-11-10T09:00:00Z", "ended_at": "2026-11-10T10:00:00Z", "unassigned_seconds": unassigned},
        "windows": case.windows.iter().enumerate().map(|(i, w)| json!({
            "index": i, "app": w.app, "title": w.title, "site": w.site, "seconds": w.seconds,
            "result": w.result, "matched_goal_id": w.goal, "visible_text": w.text,
        })).collect::<Vec<_>>(),
        "goals": case.goals.iter().map(|g| json!({
            "id": g.id, "title": g.title, "user_done_when": "",
            "steps": g.steps.iter().map(|s| json!({"id": s.id, "text": s.text, "done": false})).collect::<Vec<_>>(),
            "meaning": g.meaning, "outcomes": null, "targets": g.targets, "in_today": true,
        })).collect::<Vec<_>>(),
    });
    Job {
        key: case.id.clone(),
        windows,
        goals: case.goals.iter().map(|g| g.id).collect(),
        steps: case
            .goals
            .iter()
            .flat_map(|g| g.steps.iter().map(|s| (g.id, s.id.clone())))
            .collect(),
        revisions: Default::default(),
        unassigned_seconds: unassigned,
        context,
    }
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!(
        "../../test-data/episode-review-benchmark.json"
    ))
    .unwrap()
}

#[test]
fn the_benchmark_holds_twenty_near_misses_and_scores_like_the_app() {
    let cases = cases();
    let ids: BTreeSet<_> = cases.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), cases.len(), "case ids are unique");
    let near_misses = cases
        .iter()
        .filter(|c| c.expect.complete.is_empty() && !c.expect.new_goal)
        .count();
    assert!(near_misses >= 20, "at least 20 false-suggestion traps");
    for case in &cases {
        let job = job(case);
        // The ideal answer for each case passes validation and the app's thresholds.
        let ideal = Answer {
            windows: case
                .expect
                .attribute
                .iter()
                .map(|(i, g)| WindowAnswer {
                    index: i.parse().unwrap(),
                    goal_id: Some(*g),
                    confidence: 0.9,
                })
                .collect(),
            outcomes: case
                .expect
                .complete
                .iter()
                .map(|c| {
                    let (goal, step) = match c.split_once(':') {
                        Some((g, s)) => (g.parse().unwrap(), Some(s.to_string())),
                        None => (c.parse().unwrap(), None),
                    };
                    OutcomeAnswer {
                        goal_id: goal,
                        step_id: step,
                        status: "achieved".into(),
                        outcome: "did it".into(),
                        evidence: vec![0],
                        confidence: 0.9,
                    }
                })
                .collect(),
            new_goal: case.expect.new_goal.then(|| NewGoal {
                title: "Find an apartment in Lisbon".into(),
                evidence: vec![0],
                confidence: 0.9,
            }),
        };
        validate(&job, &ideal).unwrap_or_else(|e| panic!("{}: {e}", case.id));
        assert!(passes(case, &effect(&job, &ideal)), "{}", case.id);
        // A weak or evidence-free "achieved" is never a card.
        let weak = Answer {
            windows: vec![],
            outcomes: vec![OutcomeAnswer {
                goal_id: case.goals[0].id,
                step_id: None,
                status: "achieved".into(),
                outcome: "spent time on it".into(),
                evidence: vec![],
                confidence: 0.95,
            }],
            new_goal: None,
        };
        assert!(effect(&job, &weak).complete.is_empty());
    }
}

/// Opt-in real-model benchmark: `SUMI_LIVE_BENCHMARK=1 cargo test --locked --lib --
/// --ignored episode_review_live_benchmark --nocapture`, or `npm run bench:live`.
/// Uses real HTTP through the configured provider. Never part of default CI.
#[tokio::test]
#[ignore = "real provider calls; run explicitly"]
async fn episode_review_live_benchmark() {
    if std::env::var("SUMI_LIVE_BENCHMARK").as_deref() != Ok("1") {
        eprintln!("skipped: set SUMI_LIVE_BENCHMARK=1");
        return;
    }
    let state = crate::commands::AppState::new(
        crate::storage::Storage::open(std::path::Path::new(":memory:")).unwrap(),
    )
    .unwrap();
    let cases = cases();
    let (mut passed, mut false_cards, mut missed, mut errors) = (0, 0, 0, 0);
    for case in &cases {
        let job = job(case);
        let started = Instant::now();
        let result = request(&state, &job)
            .await
            .and_then(|a| validate(&job, &a).map(|_| a));
        let latency = started.elapsed().as_millis();
        let Ok(answer) = result else {
            errors += 1;
            eprintln!("{}: ERROR in {latency} ms {:?}", case.id, result.err());
            continue;
        };
        let got = effect(&job, &answer);
        let expected: BTreeSet<String> = case.expect.complete.iter().cloned().collect();
        false_cards += got.complete.difference(&expected).count()
            + usize::from(got.new_goal && !case.expect.new_goal);
        missed += expected.difference(&got.complete).count()
            + usize::from(!got.new_goal && case.expect.new_goal);
        let ok = passes(case, &got);
        passed += usize::from(ok);
        eprintln!(
            "{}: {} in {latency} ms {:?}",
            case.id,
            if ok { "pass" } else { "FAIL" },
            got
        );
    }
    eprintln!(
        "episode review: {passed}/{} passed, {false_cards} false cards, {missed} missed, {errors} errors",
        cases.len()
    );
}
