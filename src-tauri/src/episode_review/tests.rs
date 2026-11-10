use super::*;
use crate::{commands::AppState, core::GoalDraft, models::ActivitySnapshot, storage::Storage};

fn setup(titles: &[&str]) -> (AppState, Vec<i64>) {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    for (i, title) in titles.iter().enumerate() {
        s.core_add_goals(
            vec![GoalDraft {
                title: (*title).into(),
                area: "Work".into(),
                steps: vec![],
            }],
            &format!("review-{i}"),
        )
        .unwrap();
    }
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
        i.companion.view.preferences.screen_task_detection = true;
    }
    (state, ids)
}
fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}
fn record(i: &Inner, app: &str, title: &str, from: &str, seconds: i64, goal: Option<i64>) {
    let mut t = at(from);
    let end = t + chrono::Duration::seconds(seconds);
    while t < end {
        let activity = ActivitySnapshot {
            process_name: app.into(),
            window_title: title.into(),
            window_id: Some(title.len() as u64),
            timestamp: t.to_rfc3339(),
            ..Default::default()
        };
        i.storage
            .record_review_interval(
                &crate::history::UsageInterval {
                    start: t,
                    end: t + chrono::Duration::seconds(3),
                    activity,
                    state: crate::tracking::ActivityState::Focused,
                },
                &crate::relevance::ActivityMatch {
                    goal_id: goal.map(|g| g.to_string()),
                    confidence: if goal.is_some() { 0.9 } else { 0. },
                    reason: "test".into(),
                },
            )
            .unwrap();
        t += chrono::Duration::seconds(3);
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
fn window(index: usize, goal: Option<i64>, confidence: f64) -> WindowAnswer {
    WindowAnswer {
        index,
        goal_id: goal,
        confidence,
    }
}
fn achieved(goal: i64, evidence: Vec<usize>, confidence: f64) -> OutcomeAnswer {
    OutcomeAnswer {
        goal_id: goal,
        step_id: None,
        status: "achieved".into(),
        outcome: "sent your interview availability".into(),
        evidence,
        confidence,
    }
}

#[test]
fn an_episode_is_reviewed_once_after_goal_work_stops_and_never_while_it_continues() {
    let (state, ids) = setup(&["Sort out the interview"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    record(
        &i,
        "Mail",
        "Interview availability",
        "2026-11-10T09:00:00Z",
        90,
        Some(ids[0]),
    );
    record(&i, "Telegram", "Friends", "2026-11-10T09:01:30Z", 60, None);
    assert!(
        claim(&mut i, base, at("2026-11-10T09:02:30Z"))
            .unwrap()
            .is_none(),
        "goal work stopped only a minute ago"
    );
    record(&i, "Telegram", "Friends", "2026-11-10T09:02:30Z", 120, None);
    let job = claim(&mut i, base, at("2026-11-10T09:04:30Z"))
        .unwrap()
        .unwrap();
    assert_eq!(job.windows.len(), 2);
    assert_eq!(job.context["windows"][0]["matched_goal_id"], ids[0]);
    finish(
        &mut i,
        &job,
        Ok(Answer {
            windows: vec![],
            outcomes: vec![],
            new_goal: None,
        }),
        base,
    )
    .unwrap();
    let later = base + MIN_INTERVAL;
    assert!(
        claim(&mut i, later, at("2026-11-10T09:05:00Z"))
            .unwrap()
            .is_none(),
        "the same episode is not reviewed twice"
    );
}

#[test]
fn short_episodes_and_missing_consent_are_never_sent() {
    let (state, ids) = setup(&["Sort out the interview"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    record(
        &i,
        "Mail",
        "Interview availability",
        "2026-11-10T09:00:00Z",
        30,
        Some(ids[0]),
    );
    assert!(
        claim(&mut i, base, at("2026-11-10T09:10:00Z"))
            .unwrap()
            .is_none(),
        "too short"
    );
    record(
        &i,
        "Mail",
        "Interview availability",
        "2026-11-10T10:00:00Z",
        120,
        Some(ids[0]),
    );
    i.companion.view.preferences.screen_task_detection = false;
    assert!(
        claim(&mut i, base, at("2026-11-10T10:10:00Z"))
            .unwrap()
            .is_none(),
        "no consent"
    );
    i.companion.view.preferences.screen_task_detection = true;
    i.status.anthropic_configured = false;
    assert!(
        claim(&mut i, base, at("2026-11-10T10:10:00Z"))
            .unwrap()
            .is_none(),
        "no provider"
    );
    assert!(
        !reviews_episodes(&i),
        "the time-based offer is the fallback"
    );
}

#[test]
fn answers_naming_unknown_windows_goals_or_steps_are_rejected() {
    let (state, ids) = setup(&["Sort out the interview"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    record(
        &i,
        "Mail",
        "Interview availability",
        "2026-11-10T09:00:00Z",
        90,
        None,
    );
    let job = claim(&mut i, base, at("2026-11-10T09:10:00Z"))
        .unwrap()
        .unwrap();
    let bad = [
        Answer {
            windows: vec![window(5, Some(ids[0]), 0.9)],
            outcomes: vec![],
            new_goal: None,
        },
        Answer {
            windows: vec![window(0, Some(999), 0.9)],
            outcomes: vec![],
            new_goal: None,
        },
        Answer {
            windows: vec![],
            outcomes: vec![OutcomeAnswer {
                step_id: Some("nope".into()),
                ..achieved(ids[0], vec![0], 0.9)
            }],
            new_goal: None,
        },
        Answer {
            windows: vec![],
            outcomes: vec![OutcomeAnswer {
                status: "maybe".into(),
                ..achieved(ids[0], vec![0], 0.9)
            }],
            new_goal: None,
        },
        Answer {
            windows: vec![],
            outcomes: vec![achieved(ids[0], vec![3], 0.9)],
            new_goal: None,
        },
    ];
    for answer in bad {
        assert!(validate(&job, &answer).is_err());
    }
    finish(&mut i, &job, Err("Episode review timed out".into()), base).unwrap();
    assert_eq!(codes(&i), vec!["provider_timeout_or_network"]);
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 0);
}

#[test]
fn a_supported_result_is_proposed_with_its_reason_and_weak_ones_are_not() {
    let (state, ids) = setup(&["Sort out the interview", "Plan the trip"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    record(
        &i,
        "Mail",
        "Interview availability",
        "2026-11-10T09:00:00Z",
        60,
        Some(ids[0]),
    );
    record(
        &i,
        "Telegram",
        "Recruiter chat",
        "2026-11-10T09:01:00Z",
        60,
        None,
    );
    record(&i, "Telegram", "Friends", "2026-11-10T09:02:00Z", 200, None);
    let job = claim(&mut i, base, at("2026-11-10T09:10:00Z"))
        .unwrap()
        .unwrap();
    let answer = Answer {
        // The recruiter chat served the goal; the friends chat did not.
        windows: vec![
            window(0, Some(ids[0]), 0.95),
            window(1, Some(ids[0]), 0.9),
            window(2, None, 0.1),
        ],
        outcomes: vec![
            achieved(ids[0], vec![0, 1], 0.85),
            // Time spent alone, without evidence, never proposes anything.
            achieved(ids[1], vec![], 0.9),
            OutcomeAnswer {
                status: "unclear".into(),
                ..achieved(ids[1], vec![2], 0.5)
            },
        ],
        new_goal: Some(NewGoal {
            title: "Catch up with friends".into(),
            evidence: vec![2],
            confidence: 0.6,
        }),
    };
    finish(&mut i, &job, Ok(answer), base).unwrap();
    assert_eq!(
        codes(&i),
        vec!["episode_reviewed", "outcome_probable", "outcome_unclear"]
    );
    // The messenger window that served the goal now counts; the unrelated one stays pending.
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 120);
    let pending: i64 = i
        .storage
        .connection
        .query_row(
            "SELECT SUM(milliseconds)/1000 FROM review_intervals WHERE status='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 201);
    assert_eq!(
        i.storage
            .core_work_snapshot()
            .unwrap()
            .goals
            .iter()
            .filter(|g| g.status == "open")
            .count(),
        2,
        "a review never closes a goal by itself"
    );
}

#[test]
fn substantial_unlisted_work_is_offered_as_a_new_goal() {
    let (state, ids) = setup(&["Sort out the interview"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    record(
        &i,
        "Excel",
        "Q4 budget.xlsx",
        "2026-11-10T09:00:00Z",
        240,
        None,
    );
    let job = claim(&mut i, base, at("2026-11-10T09:10:00Z"))
        .unwrap()
        .unwrap();
    let answer = Answer {
        windows: vec![window(0, None, 0.2)],
        outcomes: vec![],
        new_goal: Some(NewGoal {
            title: "Prepare the Q4 budget".into(),
            evidence: vec![0],
            confidence: 0.9,
        }),
    };
    finish(&mut i, &job, Ok(answer), base).unwrap();
    assert_eq!(codes(&i), vec!["episode_reviewed", "proposal_queued"]);
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 0);
}
