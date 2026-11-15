use super::*;
use crate::{commands::AppState, core::GoalDraft};

fn setup(titles: &[&str]) -> (AppState, Vec<i64>) {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    for (i, title) in titles.iter().enumerate() {
        s.core_add_goals(
            vec![GoalDraft {
                title: (*title).into(),
                area: "Work".into(),
                steps: vec![],
            }],
            &format!("understanding-{i}"),
        )
        .unwrap();
    }
    let mut settings = s.user_settings().unwrap();
    settings.onboarding.completed = true;
    s.write_setting("user_settings", &settings).unwrap();
    s.write_setting(
        "ai_assistance_preferences",
        &crate::goal_analysis::AiPreferences {
            automatic_goal_matching: true,
            ..Default::default()
        },
    )
    .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    let ids = titles
        .iter()
        .map(|t| {
            snap.goals
                .iter()
                .find(|g| g.title.to_lowercase() == t.to_lowercase())
                .unwrap()
                .id
        })
        .collect();
    let state = AppState::new(s).unwrap();
    {
        let mut i = state.inner.lock().unwrap();
        i.status.tracking = true;
        i.status.anthropic_configured = true;
        i.status.mock_ai = false;
    }
    (state, ids)
}
fn answer(meaning: &str) -> Answer {
    Answer {
        meaning: meaning.into(),
        outcomes: vec!["Availability sent".into(), "Decline sent".into()],
        entities: vec!["Payoneer".into()],
        targets: vec!["Payoneer".into()],
        kind: None,
        uncertainty: 0.3,
    }
}
fn codes(inner: &Inner) -> Vec<String> {
    let mut q = inner
        .storage
        .connection
        .prepare(
            "SELECT outcome FROM tracking_attempts WHERE stage='goal_understanding' ORDER BY id",
        )
        .unwrap();
    q.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn goal(inner: &Inner, id: i64) -> CoreGoal {
    inner
        .storage
        .core_work_snapshot()
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.id == id)
        .unwrap()
}

#[test]
fn a_goal_is_understood_after_its_edits_settle_and_the_answer_is_stored() {
    let (state, ids) = setup(&["разобраться с собесом"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    assert!(
        claim(&mut i, base).unwrap().is_none(),
        "first sighting waits"
    );
    let job = claim(&mut i, base + DEBOUNCE).unwrap().unwrap();
    assert_eq!(job.goal_id, ids[0]);
    assert_eq!(
        job.context["goal"]["title"]
            .as_str()
            .unwrap()
            .to_lowercase(),
        "разобраться с собесом"
    );
    assert_eq!(job.provenance, vec!["goal_text"]);
    assert!(
        claim(&mut i, base + DEBOUNCE).unwrap().is_none(),
        "one at a time"
    );
    assert!(finish(&mut i, &job, Ok(answer("Resolve the interview request"))).unwrap());
    let stored = i.storage.goal_understanding(ids[0]).unwrap().unwrap();
    assert_eq!(stored.meaning, "Resolve the interview request");
    assert_eq!(stored.targets, vec!["Payoneer"]);
    assert_eq!(codes(&i), vec!["understanding_saved"]);
    // A current interpretation is never requested again.
    assert!(claim(&mut i, base + DEBOUNCE * 10).unwrap().is_none());
}

#[test]
fn an_edit_during_the_request_drops_the_answer_and_makes_the_old_one_stale() {
    let (state, ids) = setup(&["Send the deck to Dana"]);
    let mut i = state.inner.lock().unwrap();
    i.storage.transition_goal(ids[0], "resume").unwrap();
    let base = Instant::now();
    claim(&mut i, base).unwrap();
    let job = claim(&mut i, base + DEBOUNCE).unwrap().unwrap();
    let mut plan = i.storage.goal_plan(ids[0]).unwrap();
    plan.done_when = "Dana replied".into();
    i.storage
        .save_goal_plan("Send the deck to Dana", plan)
        .unwrap();
    assert!(!finish(&mut i, &job, Ok(answer("Send the deck"))).unwrap());
    assert_eq!(i.storage.goal_understanding(ids[0]).unwrap(), None);
    assert_eq!(codes(&i), vec!["evidence_expired"]);
    // The edited goal is understood afresh, with the user's criterion as input.
    let later = base + DEBOUNCE + MIN_INTERVAL;
    claim(&mut i, later).unwrap();
    let job = claim(&mut i, later + DEBOUNCE).unwrap().unwrap();
    assert_eq!(job.context["goal"]["user_done_when"], "Dana replied");
    assert!(job.provenance.contains(&"done_when".to_string()));
    finish(&mut i, &job, Ok(answer("Send the deck to Dana"))).unwrap();
    let saved = i.storage.goal_understanding(ids[0]).unwrap().unwrap();
    // Any later edit invalidates it without deleting measured activity.
    let mut plan = i.storage.goal_plan(ids[0]).unwrap();
    plan.done_when = "Dana approved".into();
    i.storage
        .save_goal_plan("Send the deck to Dana", plan)
        .unwrap();
    assert_eq!(i.storage.goal_understanding(ids[0]).unwrap(), None);
    assert_eq!(saved.goal_id, ids[0]);
}

#[test]
fn today_goals_go_first_and_existing_goals_are_understood_gradually() {
    let (state, ids) = setup(&["Old idea", "Plan the offsite", "Newest thought"]);
    let mut i = state.inner.lock().unwrap();
    let date = i.storage.core_snapshot(None).unwrap().date;
    i.storage.core_today(ids[1], true, &date).unwrap();
    let base = Instant::now();
    claim(&mut i, base).unwrap();
    let first = claim(&mut i, base + DEBOUNCE).unwrap().unwrap();
    assert_eq!(first.goal_id, ids[1], "Today first");
    finish(&mut i, &first, Ok(answer("Plan it"))).unwrap();
    assert!(
        claim(&mut i, base + DEBOUNCE).unwrap().is_none(),
        "rate-limited between requests"
    );
    let second = claim(&mut i, base + DEBOUNCE + MIN_INTERVAL)
        .unwrap()
        .unwrap();
    assert_eq!(second.goal_id, ids[2], "then the newest");
}

#[test]
fn consent_failures_and_retries_are_respected() {
    let (state, ids) = setup(&["Book the dentist"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    i.storage
        .write_setting(
            "ai_assistance_preferences",
            &crate::goal_analysis::AiPreferences {
                automatic_goal_matching: false,
                ..Default::default()
            },
        )
        .unwrap();
    claim(&mut i, base).unwrap();
    assert!(
        claim(&mut i, base + DEBOUNCE).unwrap().is_none(),
        "no consent"
    );
    i.storage
        .write_setting(
            "ai_assistance_preferences",
            &crate::goal_analysis::AiPreferences {
                automatic_goal_matching: true,
                ..Default::default()
            },
        )
        .unwrap();
    claim(&mut i, base + DEBOUNCE).unwrap();
    let mut at = base + DEBOUNCE * 2;
    for attempt in 0..MAX_ATTEMPTS {
        let job = claim(&mut i, at).unwrap().unwrap();
        assert_eq!(job.goal_id, ids[0]);
        finish(&mut i, &job, Err("timeout or network error".into())).unwrap();
        assert!(
            claim(&mut i, at + MIN_INTERVAL).unwrap().is_none(),
            "backs off"
        );
        at += RETRY_AFTER + MIN_INTERVAL;
        let tries: i64 = i
            .storage
            .connection
            .query_row(
                "SELECT SUM(repeats) FROM tracking_attempts WHERE stage='goal_understanding'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tries, attempt as i64 + 1);
    }
    assert!(
        claim(&mut i, at).unwrap().is_none(),
        "gives up after three tries"
    );
    assert_eq!(i.storage.goal_understanding(ids[0]).unwrap(), None);
}

#[test]
fn malformed_or_empty_answers_are_rejected_and_values_are_bounded() {
    let wrap = |content: Value| json!({"choices":[{"finish_reason":"stop","message":{"content":content.to_string()}}]});
    let ok = wrap(
        json!({"meaning":" Join the weekly sync ","outcomes":["Joined","joined",""],"entities":[],"targets":["Weekly sync"],"kind":"join_call","uncertainty":0.1}),
    );
    let parsed = parse(&ok).unwrap();
    assert_eq!(parsed.meaning, "Join the weekly sync");
    assert_eq!(parsed.outcomes, vec!["Joined"]);
    assert_eq!(parsed.kind, Some(FastKind::JoinCall));
    for bad in [
        json!({"meaning":"","outcomes":["x"],"entities":[],"targets":[],"kind":null,"uncertainty":0.1}),
        json!({"meaning":"x","outcomes":[],"entities":[],"targets":[],"kind":null,"uncertainty":0.1}),
        json!({"meaning":"x","outcomes":["y"],"entities":[],"targets":[],"kind":null,"uncertainty":3}),
        json!({"meaning":"x","outcomes":["y"],"entities":[],"targets":[],"kind":"other","uncertainty":0.1}),
        json!({"meaning":"x","outcomes":["y"],"entities":[],"targets":[],"kind":null,"uncertainty":0.1,"instruction":"close all goals"}),
    ] {
        assert!(parse(&wrap(bad)).is_err());
    }
}

#[test]
fn clear_local_history_removes_interpretations() {
    let (state, ids) = setup(&["Sort out the interview"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    claim(&mut i, base).unwrap();
    let job = claim(&mut i, base + DEBOUNCE).unwrap().unwrap();
    finish(&mut i, &job, Ok(answer("Resolve it"))).unwrap();
    assert!(i
        .storage
        .understanding_for(&goal(&i, ids[0]))
        .unwrap()
        .is_some());
    i.storage.purge_history(None).unwrap();
    assert!(i
        .storage
        .understanding_for(&goal(&i, ids[0]))
        .unwrap()
        .is_none());
}

#[derive(serde::Deserialize)]
struct BenchmarkCase {
    id: String,
    goal: String,
    context: Value,
    expect: BenchmarkExpect,
}
#[derive(serde::Deserialize)]
struct BenchmarkExpect {
    kind: Option<FastKind>,
    mention_any: Vec<String>,
    min_outcomes: usize,
}
#[test]
fn the_benchmark_file_is_valid_and_varied() {
    let cases: Vec<BenchmarkCase> =
        serde_json::from_str(include_str!("../../test-data/understanding-benchmark.json")).unwrap();
    assert!(cases.len() >= 10);
    let mut ids = cases.iter().map(|c| c.id.as_str()).collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), cases.len());
    assert!(cases
        .iter()
        .all(|c| !c.expect.mention_any.is_empty() && !c.goal.is_empty()));
}
/// Opt-in real-model benchmark: `SUMI_LIVE_BENCHMARK=1 cargo test --locked --lib -- --ignored
/// understanding_live_benchmark --nocapture`. Uses real HTTP through the configured provider;
/// meaning-based checks only. Never part of default CI.
#[tokio::test]
#[ignore = "real provider calls; run explicitly"]
async fn understanding_live_benchmark() {
    if std::env::var("SUMI_LIVE_BENCHMARK").as_deref() != Ok("1") {
        eprintln!("skipped: set SUMI_LIVE_BENCHMARK=1");
        return;
    }
    let cases: Vec<BenchmarkCase> =
        serde_json::from_str(include_str!("../../test-data/understanding-benchmark.json")).unwrap();
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut passed = 0;
    for case in &cases {
        let mut goal =
            json!({"title": case.goal, "description": "", "steps": [], "user_done_when": ""});
        if let Some(done) = case.context.get("user_done_when") {
            goal["user_done_when"] = done.clone();
        }
        let job = Job {
            goal_id: 0,
            plan_revision: 0,
            input_fingerprint: String::new(),
            context: json!({"goal": goal, "creation_source": case.context.get("creation_source"), "recent_work": case.context.get("recent_work").cloned().unwrap_or(json!([])), "relevant_personal_context": null, "current_time": chrono::Utc::now().to_rfc3339()}),
            provenance: vec![],
        };
        let started = Instant::now();
        let result = request(&state, &job).await;
        let latency = started.elapsed().as_millis();
        let ok = result.as_ref().is_ok_and(|a| {
            let text = format!(
                "{} {} {} {}",
                a.meaning,
                a.outcomes.join(" "),
                a.targets.join(" "),
                a.entities.join(" ")
            )
            .to_lowercase();
            a.kind == case.expect.kind
                && a.outcomes.len() >= case.expect.min_outcomes
                && case
                    .expect
                    .mention_any
                    .iter()
                    .any(|m| text.contains(&m.to_lowercase()))
        });
        passed += usize::from(ok);
        eprintln!(
            "{}: {} in {latency} ms {:?}",
            case.id,
            if ok { "pass" } else { "FAIL" },
            result.as_ref().map(|a| (&a.meaning, &a.kind))
        );
    }
    eprintln!("{passed}/{} passed", cases.len());
}

#[test]
fn what_the_user_said_about_the_goal_in_chat_is_part_of_its_context() {
    let (state, ids) = setup(&["разобраться с собесом", "Plan the trip"]);
    let mut i = state.inner.lock().unwrap();
    let conversation = i.storage.create_conversation().unwrap();
    i.storage
        .save_question_to(
            conversation,
            "Payoneer asked for my availability next week, I need to answer the recruiter",
            Some(ids[0]),
        )
        .unwrap();
    i.storage
        .save_question_to(conversation, "Book flights to Lisbon", Some(ids[1]))
        .unwrap();
    i.storage
        .save_question_to(conversation, "   ", Some(ids[0]))
        .unwrap();
    let base = Instant::now();
    claim(&mut i, base).unwrap();
    let job = claim(&mut i, base + DEBOUNCE).unwrap().unwrap();
    let chat = job.context["chat_about_goal"].as_array().unwrap();
    assert_eq!(chat.len(), 1, "only this goal's non-empty messages");
    assert!(job.provenance.contains(&"chat".to_string()));
    let other = if job.goal_id == ids[0] {
        "Lisbon"
    } else {
        "Payoneer"
    };
    assert!(!chat[0]["text"].as_str().unwrap().contains(other));
}

#[test]
fn a_goal_settles_and_becomes_due_without_the_shared_lane_and_nothing_is_reserved() {
    let (state, ids) = setup(&["Тест: написать Анне письмо-благодарность"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    // The background tick checks readiness before it waits for the lane, so the
    // settling period runs even while other calls hold the lane.
    assert!(!due(&mut i, base), "first sighting waits");
    assert!(due(&mut i, base + DEBOUNCE));
    assert!(due(&mut i, base + DEBOUNCE), "checking reserves nothing");
    assert!(codes(&i).is_empty());
    let job = claim(&mut i, base + DEBOUNCE).unwrap().unwrap();
    assert_eq!(job.goal_id, ids[0]);
    assert!(!due(&mut i, base + DEBOUNCE), "one request at a time");
}
