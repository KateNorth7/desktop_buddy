use super::*;
use crate::{commands::AppState, core::GoalDraft, storage::Storage};

fn setup(goals: &[(&str, &[&str])]) -> (AppState, Vec<i64>) {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    for (i, (title, steps)) in goals.iter().enumerate() {
        s.core_add_goals(
            vec![GoalDraft {
                title: (*title).into(),
                area: "Work".into(),
                steps: steps.iter().map(|s| s.to_string()).collect(),
            }],
            &format!("decision-{i}"),
        )
        .unwrap();
    }
    let snap = s.core_snapshot(None).unwrap();
    let ids = goals
        .iter()
        .map(|(t, _)| {
            snap.goals
                .iter()
                .find(|g| g.title.to_lowercase() == t.to_lowercase())
                .unwrap()
                .id
        })
        .collect();
    let state = AppState::new(s).unwrap();
    state
        .inner
        .lock()
        .unwrap()
        .companion
        .view
        .preferences
        .screen_task_detection = true;
    (state, ids)
}
fn decision(goal: Option<i64>, kind: DecisionKind, key: &str, source: &str) -> Decision {
    Decision {
        goal_id: goal,
        step_id: None,
        kind,
        evidence: vec![],
        reason: "Looks like you sent it.".into(),
        confidence: 0.8,
        dedupe_key: key.into(),
        source: source.into(),
        text: None,
        place: None,
        plan_revision: None,
    }
}
fn codes(inner: &Inner) -> Vec<String> {
    let mut q = inner
        .storage
        .connection
        .prepare("SELECT outcome FROM tracking_attempts ORDER BY id")
        .unwrap();
    q.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn status(inner: &Inner, id: i64) -> String {
    inner
        .storage
        .core_work_snapshot()
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.id == id)
        .map(|g| g.status)
        .unwrap()
}

#[test]
fn one_goal_is_offered_once_whichever_analyser_asks() {
    let (state, ids) = setup(&[("Send the deck", &[])]);
    let mut i = state.inner.lock().unwrap();
    let now = Instant::now();
    let first = submit_at(
        &mut i,
        decision(
            Some(ids[0]),
            DecisionKind::ProposeComplete,
            "a",
            "fast_signal",
        ),
        now,
    )
    .unwrap();
    assert_eq!(first, Applied::Queued);
    // The same finding again, and another analyser's finding within the cooldown.
    assert_eq!(
        submit_at(
            &mut i,
            decision(
                Some(ids[0]),
                DecisionKind::ProposeComplete,
                "a",
                "fast_signal"
            ),
            now
        )
        .unwrap(),
        Applied::Deduplicated
    );
    assert_eq!(
        submit_at(
            &mut i,
            decision(
                Some(ids[0]),
                DecisionKind::ProposeComplete,
                "b",
                "episode_review"
            ),
            now
        )
        .unwrap(),
        Applied::Deduplicated
    );
    assert_eq!(status(&i, ids[0]), "open", "a proposal never closes a goal");
    assert_eq!(codes(&i), vec!["outcome_probable", "proposal_deduplicated"]);
    // After the cooldown a new finding may ask again.
    let later = now + COMPLETE_COOLDOWN;
    assert_eq!(
        submit_at(
            &mut i,
            decision(
                Some(ids[0]),
                DecisionKind::ProposeComplete,
                "c",
                "episode_review"
            ),
            later
        )
        .unwrap(),
        Applied::Queued
    );
}

#[test]
fn changed_goals_done_steps_open_steps_and_missing_consent_are_rejected() {
    let (state, ids) = setup(&[("Plan the offsite", &["Book the venue"]), ("Old goal", &[])]);
    let mut i = state.inner.lock().unwrap();
    let now = Instant::now();
    // The goal card cannot close a goal with an unfinished step.
    assert_eq!(
        submit_at(
            &mut i,
            decision(
                Some(ids[0]),
                DecisionKind::ProposeComplete,
                "g",
                "episode_review"
            ),
            now
        )
        .unwrap(),
        Applied::Rejected("open_steps")
    );
    // An unknown step is stale evidence; the real step is proposed with the step card.
    let mut stale = decision(
        Some(ids[0]),
        DecisionKind::ProposeComplete,
        "s1",
        "episode_review",
    );
    stale.step_id = Some("missing".into());
    assert_eq!(
        submit_at(&mut i, stale, now).unwrap(),
        Applied::Rejected("step_changed")
    );
    let step = i.storage.goal_plan(ids[0]).unwrap().steps[0].id.clone();
    let mut real = decision(
        Some(ids[0]),
        DecisionKind::ProposeComplete,
        "s2",
        "episode_review",
    );
    real.step_id = Some(step);
    assert_eq!(submit_at(&mut i, real, now).unwrap(), Applied::Queued);
    // A finding about an older plan revision is dropped.
    let mut old = decision(
        Some(ids[0]),
        DecisionKind::ProposeComplete,
        "r",
        "episode_review",
    );
    old.step_id = i
        .storage
        .goal_plan(ids[0])
        .unwrap()
        .steps
        .first()
        .map(|s| s.id.clone());
    old.plan_revision = Some(i.storage.goal_plan(ids[0]).unwrap().revision - 1);
    assert_eq!(
        submit_at(&mut i, old, now).unwrap(),
        Applied::Rejected("plan_changed")
    );
    // A closed goal is no longer a target.
    i.storage.core_transition(ids[1], "complete").unwrap();
    assert_eq!(
        submit_at(
            &mut i,
            decision(
                Some(ids[1]),
                DecisionKind::ProposeComplete,
                "c",
                "episode_review"
            ),
            now
        )
        .unwrap(),
        Applied::Rejected("goal_changed")
    );
    // Without interface consent nothing is proposed.
    i.companion.view.preferences.screen_task_detection = false;
    assert_eq!(
        submit_at(
            &mut i,
            decision(None, DecisionKind::ProposeGoal, "n", "episode_review"),
            now
        )
        .unwrap(),
        Applied::Rejected("consent")
    );
}

#[test]
fn automatic_completion_respects_the_setting_and_keeps_a_receipt() {
    let (state, ids) = setup(&[("Join the design sync", &[])]);
    let mut i = state.inner.lock().unwrap();
    let now = Instant::now();
    i.companion.view.preferences.automatic_completion = false;
    let mut auto = decision(
        Some(ids[0]),
        DecisionKind::AutoComplete,
        "auto-1",
        "fast_signal",
    );
    auto.place = Some("zoom.us".into());
    assert_eq!(
        submit_at(&mut i, auto.clone(), now).unwrap(),
        Applied::Queued
    );
    assert_eq!(status(&i, ids[0]), "open");
    i.companion.view.preferences.automatic_completion = true;
    let mut again = auto.clone();
    again.dedupe_key = "auto-2".into();
    let later = now + COMPLETE_COOLDOWN;
    assert_eq!(submit_at(&mut i, again, later).unwrap(), Applied::Changed);
    assert_eq!(status(&i, ids[0]), "completed");
    let receipts: i64 = i
        .storage
        .connection
        .query_row(
            "SELECT COUNT(*) FROM work_events WHERE status='completed' AND undo_plan IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 1, "a durable Undo exists");
    assert_eq!(
        codes(&i),
        vec!["automatic_completion_disabled", "completion_applied"]
    );
}

#[test]
fn new_goals_and_steps_need_text_are_not_duplicated_and_are_offered_once() {
    let (state, ids) = setup(&[("Find a dentist", &["Call the clinic"])]);
    let mut i = state.inner.lock().unwrap();
    let now = Instant::now();
    let mut goal = decision(None, DecisionKind::ProposeGoal, "g1", "episode_review");
    goal.text = Some("find a dentist".into());
    assert_eq!(
        submit_at(&mut i, goal.clone(), now).unwrap(),
        Applied::Rejected("already_listed")
    );
    goal.text = Some("Renew the passport".into());
    assert_eq!(
        submit_at(&mut i, goal.clone(), now).unwrap(),
        Applied::Queued
    );
    goal.dedupe_key = "g2".into();
    assert_eq!(submit_at(&mut i, goal, now).unwrap(), Applied::Deduplicated);
    let mut step = decision(
        Some(ids[0]),
        DecisionKind::ProposeStep,
        "s1",
        "episode_review",
    );
    step.text = Some("Call the clinic".into());
    assert_eq!(
        submit_at(&mut i, step.clone(), now).unwrap(),
        Applied::Rejected("already_listed")
    );
    step.text = None;
    assert_eq!(
        submit_at(&mut i, step, now).unwrap(),
        Applied::Rejected("empty_text")
    );
}

#[test]
fn attribution_moves_only_pending_time_without_double_counting() {
    let (state, ids) = setup(&[("Reply to the recruiter", &[]), ("Other", &[])]);
    let mut i = state.inner.lock().unwrap();
    let start = chrono::DateTime::parse_from_rfc3339("2026-11-10T09:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let chat = crate::models::ActivitySnapshot {
        process_name: "Telegram".into(),
        window_title: "Recruiter".into(),
        window_id: Some(4),
        timestamp: start.to_rfc3339(),
        ..Default::default()
    };
    for n in 0..4 {
        let at = start + chrono::Duration::seconds(3 * n);
        let mut a = chat.clone();
        a.timestamp = at.to_rfc3339();
        i.storage
            .record_review_interval(
                &crate::history::UsageInterval {
                    start: at,
                    end: at + chrono::Duration::seconds(3),
                    activity: a,
                    state: crate::tracking::ActivityState::Focused,
                },
                &crate::relevance::ActivityMatch {
                    goal_id: None,
                    confidence: 0.,
                    reason: "test".into(),
                },
            )
            .unwrap();
    }
    let window = i
        .storage
        .episode_evidence(crate::episode_evidence::EpisodeRange {
            since: start,
            until: start + chrono::Duration::minutes(1),
        })
        .unwrap()
        .remove(0);
    let reference = format!("review:{}:{}", window.period_id, window.context_key);
    let mut attribute = decision(
        Some(ids[0]),
        DecisionKind::Attribute,
        "a1",
        "episode_review",
    );
    attribute.evidence = vec![reference.clone()];
    attribute.confidence = 0.9;
    assert_eq!(
        submit_at(&mut i, attribute.clone(), Instant::now()).unwrap(),
        Applied::Changed
    );
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 12);
    // Already assigned time never moves to another goal, and nothing is counted twice.
    let mut other = attribute.clone();
    other.goal_id = Some(ids[1]);
    other.dedupe_key = "a2".into();
    assert_eq!(
        submit_at(&mut i, other, Instant::now()).unwrap(),
        Applied::Skipped
    );
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 12);
    assert_eq!(i.storage.relevant_seconds(ids[1], None).unwrap(), 0);
}
