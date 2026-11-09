//! Replays of real, sanitized window sequences (`test-data/replay-cases.json`) through the
//! attribution pipeline. The matcher is replaced by an oracle that always names the right
//! goal whenever it is asked, so the measured loss belongs to the pipeline itself: when it
//! asks, what it carries across windows, and what stays pending. Model quality is measured
//! separately by the live matching benchmark.
use super::*;
use crate::{core::GoalDraft, storage::Storage};
use std::collections::HashMap;

#[derive(Deserialize)]
struct Case {
    id: String,
    goals: Vec<CaseGoal>,
    windows: Vec<Window>,
}
#[derive(Deserialize)]
struct CaseGoal {
    key: String,
    title: String,
}
#[derive(Deserialize)]
struct Window {
    app: String,
    #[serde(default)]
    domain: Option<String>,
    title: String,
    seconds: u64,
    #[serde(default)]
    goal: Option<String>,
}
/// What one replay measured, in seconds.
#[derive(Debug)]
struct Replay {
    expected: i64,
    attributed: i64,
    misattributed: i64,
    asked: usize,
}
impl Replay {
    fn share(&self) -> f64 {
        self.attributed as f64 / self.expected.max(1) as f64
    }
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("../../test-data/replay-cases.json")).unwrap()
}
fn snapshot(window: &Window) -> ActivitySnapshot {
    let browser = window.domain.is_some();
    let mut a = ActivitySnapshot {
        timestamp: chrono::Utc::now().to_rfc3339(),
        process_name: window.app.clone(),
        window_title: window.title.clone(),
        idle_seconds: 1,
        // Browser tabs share one window; every other app has its own.
        window_id: Some(if browser {
            11
        } else {
            100 + window.app.len() as u64
        }),
        ..Default::default()
    };
    if browser {
        a.browser =
            crate::browser::context(&a.process_name, &a.window_title, window.domain.clone());
    }
    a
}
fn replay(case: &Case) -> Replay {
    let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    for (i, goal) in case.goals.iter().enumerate() {
        storage
            .core_add_goals(
                vec![GoalDraft {
                    title: goal.title.clone(),
                    area: "Work".into(),
                    steps: vec![],
                }],
                &format!("replay-{i}"),
            )
            .unwrap();
    }
    let snap = storage.core_snapshot(None).unwrap();
    let ids: HashMap<String, i64> = case
        .goals
        .iter()
        .map(|goal| {
            let id = snap
                .goals
                .iter()
                .find(|g| g.title == goal.title)
                .unwrap()
                .id;
            (goal.key.clone(), id)
        })
        .collect();
    let target = ids["target"];
    // As in both real cases: the goal was accepted moments earlier and is the active lane.
    storage.core_today(target, true, &snap.date).unwrap();
    storage.transition_goal(target, "resume").unwrap();
    storage
        .write_setting(
            "ai_assistance_preferences",
            &crate::goal_analysis::AiPreferences {
                automatic_goal_matching: true,
                ..Default::default()
            },
        )
        .unwrap();
    let state = AppState::new(storage).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.status.tracking = true;
    crate::tracking::allow_notifications(&mut inner);
    inner.status.nebius_configured = true;
    inner.status.mock_ai = false;
    // The oracle knows each window's real goal by its metadata.
    let truth: HashMap<String, Option<i64>> = case
        .windows
        .iter()
        .map(|w| {
            (
                fingerprint(&snapshot(w)),
                w.goal.as_ref().map(|key| ids[key]),
            )
        })
        .collect();
    // Wall time runs forward from now with the monotonic clock, so the pipeline's own
    // "now minus elapsed" arithmetic lands on the recorded intervals.
    let base = Instant::now();
    let start = chrono::Utc::now();
    let mut clock = 0u64;
    let mut asked = 0;
    for window in &case.windows {
        let a = snapshot(window);
        let key = fingerprint(&a);
        let mut t = 0;
        while t < window.seconds {
            clock += 3;
            t += 3;
            let instant = base + Duration::from_secs(clock);
            let wall = start + chrono::Duration::seconds(clock as i64);
            observe_inner_at(&mut inner, &a, true, instant).unwrap();
            if let Some(entry) = inner
                .goal_matching
                .trail
                .back_mut()
                .filter(|e| e.key == key)
            {
                if entry.began == instant {
                    entry.at = wall.timestamp() - 3;
                }
                entry.until = wall.timestamp();
            }
            let attribution = attribution(&inner, &a).unwrap();
            let interval = crate::history::UsageInterval {
                start: wall - chrono::Duration::seconds(3),
                end: wall,
                activity: a.clone(),
                state: crate::tracking::ActivityState::Focused,
            };
            let current = inner.storage.goal().unwrap().map_or(target, |g| g.id);
            inner
                .storage
                .record_interval_with_match(current, &interval, Some(attribution.clone()))
                .unwrap();
            inner
                .storage
                .record_review_interval(&interval, &attribution)
                .unwrap();
            if let Some(job) = claim(&mut inner, instant).unwrap() {
                asked += 1;
                let goal = truth.get(&fingerprint(&job.sample)).copied().flatten();
                let trail = job
                    .trail
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| goal.is_some() && truth.get(&e.key).copied().flatten() == goal)
                    .map(|(index, _)| index)
                    .collect();
                finish(
                    &mut inner,
                    &job,
                    Ok(MatchResult {
                        goal_id: goal,
                        confidence: if goal.is_some() { 0.9 } else { 0.2 },
                        reason: "Replay oracle".into(),
                        same_task_trail: trail,
                    }),
                    instant,
                )
                .unwrap();
            }
        }
    }
    let expected = case
        .windows
        .iter()
        .filter(|w| w.goal.as_deref() == Some("target"))
        .map(|w| w.seconds.div_ceil(3) as i64 * 3)
        .sum();
    if std::env::var("REPLAY_DEBUG").is_ok() {
        let mut q = inner.storage.connection.prepare("SELECT app,title,status,COALESCE(goal_id,0),SUM(milliseconds)/1000,MIN(reason) FROM review_intervals GROUP BY context_key ORDER BY MIN(started_at)").unwrap();
        for row in q
            .query_map([], |r| {
                Ok(format!(
                    "{} | {} | {} | {} | {}s | {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?
                ))
            })
            .unwrap()
        {
            eprintln!("{}", row.unwrap());
        }
    }
    let attributed = inner.storage.relevant_seconds(target, None).unwrap() as i64;
    let misattributed = ids
        .values()
        .filter(|id| **id != target)
        .map(|id| inner.storage.relevant_seconds(*id, None).unwrap() as i64)
        .sum();
    Replay {
        expected,
        attributed,
        misattributed,
        asked,
    }
}
/// Baseline: the replays run end to end, never credit an unrelated goal, and stay within
/// the AI request budget.
#[test]
fn real_sequences_replay_without_crediting_unrelated_goals() {
    for case in cases() {
        let result = replay(&case);
        println!(
            "{}: {}s of {}s reached the goal ({:.0}%), {} matcher calls",
            case.id,
            result.attributed,
            result.expected,
            result.share() * 100.,
            result.asked
        );
        assert_eq!(result.misattributed, 0, "{}: {result:?}", case.id);
        assert!(
            result.attributed <= result.expected + 30,
            "{}: {result:?}",
            case.id
        );
        assert!(
            result.asked <= crate::observation::HOURLY_LIMIT,
            "{}: {result:?}",
            case.id
        );
    }
}
/// With a perfect matcher, nearly all of the goal's time reaches it, including short hops
/// between the email, the draft and the form (52% before episode attribution).
#[test]
fn a_perfect_matcher_attributes_nearly_all_goal_time() {
    for case in cases() {
        let result = replay(&case);
        assert!(result.share() >= 0.9, "{}: {result:?}", case.id);
    }
}
