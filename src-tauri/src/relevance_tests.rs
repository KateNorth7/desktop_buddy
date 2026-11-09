use super::*;
use crate::{history::UsageInterval, tracking::ActivityState};
use chrono::{Duration, TimeZone, Utc};
fn store() -> (Storage, i64) {
    let mut store = Storage::open(std::path::Path::new(":memory:")).unwrap();
    store
        .core_create_goal(
            "Build sumi onboarding",
            "buddy",
            &chrono::Local::now().date_naive().to_string(),
        )
        .unwrap();
    let id = store.ensure_today_focus().unwrap().unwrap();
    (store, id)
}
fn interval(start: chrono::DateTime<Utc>, title: &str, state: ActivityState) -> UsageInterval {
    UsageInterval {
        start,
        end: start + Duration::seconds(3),
        state,
        activity: ActivitySnapshot {
            process_name: "chrome.exe".into(),
            window_title: title.into(),
            browser: crate::browser::context("chrome.exe", title, Some("example.com".into())),
            ..Default::default()
        },
    }
}
#[test]
fn cloud_attribution_reaches_progress_without_local_keyword_evidence() {
    let (mut store, id) = store();
    let start = Utc::now();
    let sample = interval(start, "Editor workspace", ActivityState::Focused);
    assert!(store
        .match_activity(&sample.activity)
        .unwrap()
        .goal_id
        .is_none());
    store
        .record_interval_with_match(
            id,
            &sample,
            Some(ActivityMatch {
                goal_id: Some(id.to_string()),
                confidence: 0.94,
                reason: "Claude identified the current goal".into(),
            }),
        )
        .unwrap();
    assert_eq!(store.relevant_seconds(id, None).unwrap(), 3);
    let segments = store.activity_segments(Some(id)).unwrap();
    assert_eq!(segments[0].activity_match.goal_id, Some(id.to_string()));
    assert_eq!(segments[0].activity_match.confidence, 0.94);
    assert_eq!(store.goal_progress(id).unwrap().activities.len(), 1);
    for (index, confidence) in [0.1, f64::NAN].into_iter().enumerate() {
        store
            .record_interval_with_match(
                id,
                &interval(
                    start + Duration::seconds(3 * (index as i64 + 1)),
                    "Editor workspace",
                    ActivityState::Focused,
                ),
                Some(ActivityMatch {
                    goal_id: Some(id.to_string()),
                    confidence,
                    reason: "Uncertain".into(),
                }),
            )
            .unwrap();
    }
    assert_eq!(store.relevant_seconds(id, None).unwrap(), 3);
}
#[test]
fn relevant_totals_and_browser_sequence_exclude_idle_unrelated_and_completed_work() {
    let (mut store, id) = store();
    let start = Utc::now();
    store
        .record_interval(
            id,
            &interval(
                start,
                "sumi onboarding documentation",
                ActivityState::Focused,
            ),
        )
        .unwrap();
    store
        .record_interval(
            id,
            &interval(
                start + Duration::seconds(3),
                "News and weather",
                ActivityState::Focused,
            ),
        )
        .unwrap();
    store
        .record_interval(
            id,
            &interval(
                start + Duration::seconds(6),
                "sumi onboarding documentation",
                ActivityState::Paused,
            ),
        )
        .unwrap();
    assert_eq!(store.relevant_seconds(id, None).unwrap(), 3);
    let snapshot = store.core_snapshot(None).unwrap();
    assert_eq!(snapshot.summary.goals[0].relevant_seconds, 3);
    assert_eq!(snapshot.summary.goals[0].tracked_seconds, 6);
    let progress = store.goal_progress(id).unwrap();
    assert_eq!(progress.activities.len(), 1);
    assert_eq!(progress.activities[0].kind, "browser");
    assert_eq!(progress.active_minutes, 0.05);
    let rows = store.activity_segments(Some(id)).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows[0].activity_match.goal_id.is_none());
    assert!(rows[1].activity_match.goal_id.is_none());
    assert_eq!(
        rows[2].activity_match.goal_id.as_deref(),
        Some(id.to_string().as_str())
    );
    assert!(rows
        .iter()
        .all(|r| r.url.is_none() && r.domain.as_deref() == Some("example.com")));
    store.core_transition(id, "complete").unwrap();
    assert!(store.ensure_today_focus().unwrap().is_none());
    assert!(store
        .record_interval(
            id,
            &interval(
                start + Duration::seconds(9),
                "sumi onboarding",
                ActivityState::Focused
            )
        )
        .is_err());
    assert_eq!(store.relevant_seconds(id, None).unwrap(), 3);
}
#[test]
fn ambiguous_goals_and_distraction_rules_do_not_inflate_relevant_time() {
    let (mut store, id) = store();
    let activity = interval(Utc::now(), "sumi onboarding", ActivityState::Focused).activity;
    store
        .save_app_rule(
            id,
            "chrome.exe",
            Some(crate::insights::Category::Distraction),
        )
        .unwrap();
    assert!(store.match_activity(&activity).unwrap().goal_id.is_none());
    store.save_app_rule(id, "chrome.exe", None).unwrap();
    store
        .core_create_goal(
            "Review sumi onboarding",
            "review",
            &chrono::Local::now().date_naive().to_string(),
        )
        .unwrap();
    assert!(store.match_activity(&activity).unwrap().goal_id.is_none());
    store
        .record_interval(
            id,
            &interval(Utc::now(), "sumi onboarding", ActivityState::Focused),
        )
        .unwrap();
    assert_eq!(store.relevant_seconds(id, None).unwrap(), 0);
}
#[test]
fn relevant_daily_totals_split_at_local_midnight_and_reject_overlaps() {
    let (mut store, id) = store();
    let midnight = chrono::Local
        .with_ymd_and_hms(2026, 10, 7, 0, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let sample = interval(
        midnight - Duration::seconds(1),
        "sumi onboarding",
        ActivityState::Focused,
    );
    store.record_interval(id, &sample).unwrap();
    assert!(store.record_interval(id, &sample).is_err());
    assert_eq!(store.relevant_seconds(id, Some("2026-10-06")).unwrap(), 1);
    assert_eq!(store.relevant_seconds(id, Some("2026-10-07")).unwrap(), 2);
    // A goal without steps keeps its relevant time on the goal itself (empty step id).
    let step_total = |store: &Storage| -> u64 {
        store
            .connection
            .query_row(
                "SELECT COALESCE(SUM(milliseconds),0)/1000 FROM goal_relevant_step_daily WHERE goal_id=?1 AND step_id=''",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(step_total(&store), 3);
    store.purge_history(Some(&midnight.to_rfc3339())).unwrap();
    assert_eq!(store.relevant_seconds(id, None).unwrap(), 2);
    assert_eq!(step_total(&store), 2);
    store.purge_history(None).unwrap();
    assert_eq!(store.relevant_seconds(id, None).unwrap(), 0);
    assert_eq!(step_total(&store), 0);
}
#[test]
fn matching_can_follow_another_today_goal_without_matching_generic_work_terms() {
    let (mut store, _) = store();
    store
        .core_create_goal(
            "Renew passport application",
            "passport",
            &chrono::Local::now().date_naive().to_string(),
        )
        .unwrap();
    let next = store
        .core_snapshot(None)
        .unwrap()
        .goals
        .iter()
        .find(|g| g.title.contains("passport"))
        .unwrap()
        .id;
    let activity = interval(
        Utc::now(),
        "Passport application instructions",
        ActivityState::Focused,
    )
    .activity;
    assert_eq!(
        store.match_activity(&activity).unwrap().goal_id.as_deref(),
        Some(next.to_string().as_str())
    );
    assert!(store
        .match_activity(
            &interval(
                Utc::now(),
                "Create work plan document",
                ActivityState::Focused
            )
            .activity
        )
        .unwrap()
        .goal_id
        .is_none());
}
#[test]
fn hourly_time_splits_relevant_intervals_at_local_hour_boundaries() {
    let (mut store, id) = store();
    let start = chrono::Local
        .with_ymd_and_hms(2026, 10, 7, 9, 59, 58)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let sample = interval(start, "sumi onboarding", ActivityState::Focused);
    store.record_interval(id, &sample).unwrap();
    let hours = store.core_hourly("2026-10-07").unwrap();
    let seconds = |hour: u8| {
        hours
            .iter()
            .filter(|item| item.hour == hour && item.goal_id == id)
            .map(|item| item.seconds)
            .sum::<u64>()
    };
    assert_eq!(seconds(9), 2);
    assert_eq!(seconds(10), 1);
    assert!(store.core_hourly("2026-10-08").unwrap().is_empty());
}
