use super::*;
use crate::{history::UsageInterval, relevance::ActivityMatch, tracking::ActivityState};

fn fixture() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}
pub(super) fn add_goal(s: &mut Storage, title: &str) -> i64 {
    let count: i64 = s
        .connection
        .query_row("SELECT COUNT(*) FROM goals", [], |r| r.get(0))
        .unwrap();
    s.core_add_goals(
        vec![crate::core::GoalDraft {
            title: title.into(),
            area: "Work".into(),
            steps: vec![],
        }],
        &format!("review-test-{count}"),
    )
    .unwrap();
    s.core_work_snapshot()
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.title == title)
        .unwrap()
        .id
}
fn interval(at: &str, app: &str, title: &str) -> UsageInterval {
    let start = DateTime::parse_from_rfc3339(at)
        .unwrap()
        .with_timezone(&Utc);
    UsageInterval {
        start,
        end: start + Duration::seconds(3),
        activity: ActivitySnapshot {
            timestamp: start.to_rfc3339(),
            process_name: app.into(),
            window_title: title.into(),
            ..Default::default()
        },
        state: ActivityState::Focused,
    }
}
fn unknown() -> ActivityMatch {
    ActivityMatch {
        goal_id: None,
        confidence: 0.,
        reason: "Unknown activity".into(),
    }
}

#[test]
fn unknown_time_survives_without_a_goal_and_retry_cannot_inflate_it() {
    let s = fixture();
    let i = interval("2026-10-21T12:00:00Z", "ChatGPT", "ChatGPT");
    s.record_review_interval(&i, &unknown()).unwrap();
    s.record_review_interval(&i, &unknown()).unwrap();
    let overview = s.activity_review().unwrap();
    assert_eq!(overview.pending_seconds, 3);
    assert_eq!(overview.blocks.len(), 1);
    assert_eq!(overview.blocks[0].status, "pending");
}
#[test]
fn assignment_corrections_transfer_exact_time_and_create_an_open_goal() {
    let mut s = fixture();
    let first = add_goal(&mut s, "Prepare presentation");
    let second = add_goal(&mut s, "Update website");
    let i = interval("2026-10-21T12:00:00Z", "ChatGPT", "ChatGPT");
    s.record_review_interval(&i, &unknown()).unwrap();
    let b = s.activity_review().unwrap().blocks.remove(0);
    s.review_action(b.period_id, &b.key, "assign", Some(first), None)
        .unwrap();
    assert_eq!(s.relevant_seconds(first, None).unwrap(), 3);
    let assigned = s.activity_review().unwrap().blocks.remove(0);
    s.review_action(
        assigned.period_id,
        &assigned.key,
        "assign",
        Some(second),
        None,
    )
    .unwrap();
    assert_eq!(s.relevant_seconds(first, None).unwrap(), 0);
    assert_eq!(s.relevant_seconds(second, None).unwrap(), 3);
    let assigned = s.activity_review().unwrap().blocks.remove(0);
    s.review_action(assigned.period_id, &assigned.key, "outside", None, None)
        .unwrap();
    assert_eq!(s.relevant_seconds(second, None).unwrap(), 0);
    let outside = s.activity_review().unwrap().blocks.remove(0);
    s.review_action(
        outside.period_id,
        &outside.key,
        "create",
        None,
        Some("Organize an event"),
    )
    .unwrap();
    let created = s.activity_review().unwrap().blocks.remove(0);
    let goal = created.goal_id.unwrap();
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 3);
    let state: String = s
        .connection
        .query_row(
            "SELECT state FROM core_goals WHERE goal_id=?1",
            [goal],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "open");
    assert!(s
        .review_action(b.period_id, &b.key, "create", None, Some("Duplicate retry"))
        .is_err());
}
#[test]
fn an_existing_goal_detected_by_ai_receives_pending_time_but_not_manual_decisions() {
    let mut s = fixture();
    let goal = add_goal(&mut s, "Design checkout");
    let i = interval("2026-10-21T12:00:00Z", "Figma", "Checkout");
    s.record_review_interval(&i, &unknown()).unwrap();
    s.classify_review_context(&i.activity, Some(goal), None, "Designing checkout")
        .unwrap();
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 3);
    let b = s.activity_review().unwrap().blocks.remove(0);
    s.review_action(b.period_id, &b.key, "outside", None, None)
        .unwrap();
    s.classify_review_context(&i.activity, Some(goal), None, "Designing checkout")
        .unwrap();
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 0);
    assert_eq!(s.activity_review().unwrap().blocks[0].status, "outside");
}
#[test]
fn independently_recognized_work_groups_across_apps_without_duplicating_intervals() {
    let s = fixture();
    for (at, app, title) in [
        ("2026-10-21T12:00:00Z", "Telegram", "Event planning"),
        ("2026-10-21T12:00:06Z", "Browser", "Venue search"),
    ] {
        let i = interval(at, app, title);
        s.record_review_interval(&i, &unknown()).unwrap();
        s.classify_review_context(&i.activity, None, Some("Organize an event"), title)
            .unwrap();
    }
    let overview = s.activity_review().unwrap();
    assert_eq!(overview.pending_seconds, 6);
    assert_eq!(overview.blocks.len(), 1);
    assert!(overview.blocks[0].apps.contains("telegram"));
    assert!(overview.blocks[0].apps.contains("browser"));
}
#[test]
fn configured_summary_time_controls_the_daily_period() {
    let at = Utc.with_ymd_and_hms(2026, 10, 21, 16, 29, 0).unwrap();
    let p = period_at(at, 16 * 60 + 30).unwrap();
    assert_eq!(p.ends_at, "2026-10-21T16:30:00+00:00");
    let at = Utc.with_ymd_and_hms(2026, 10, 24, 2, 0, 0).unwrap();
    let p = period_at(at, 6 * 60).unwrap();
    assert_eq!(p.day, "2026-10-24");
    assert_eq!(p.ends_at, "2026-10-24T06:00:00+00:00");
    let at = Utc.with_ymd_and_hms(2026, 10, 24, 6, 0, 0).unwrap();
    assert_eq!(period_at(at, 6 * 60).unwrap().day, "2026-10-25");
}
#[test]
fn only_durable_summary_commit_removes_evidence_and_retries_are_idempotent() {
    let mut s = fixture();
    let i = interval("2026-10-21T12:00:00Z", "ChatGPT", "ChatGPT");
    s.record_review_interval(&i, &unknown()).unwrap();
    s.retain_review_evidence(&i.activity, "image", "c3ludGhldGljLWpwZWc=")
        .unwrap();
    let count = |s: &Storage| -> i64 {
        s.connection
            .query_row("SELECT COUNT(*) FROM review_evidence", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count(&s), 1);
    s.connection
        .execute("UPDATE review_periods SET analysis_done=1", [])
        .unwrap();
    s.connection.execute_batch("CREATE TRIGGER fail_summary BEFORE UPDATE OF summary ON review_periods BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap();
    assert!(s.finish_due_reviews(now).is_err());
    assert_eq!(count(&s), 1);
    s.connection
        .execute_batch("DROP TRIGGER fail_summary")
        .unwrap();
    s.finish_due_reviews(now).unwrap();
    s.finish_due_reviews(now).unwrap();
    assert_eq!(count(&s), 0);
    let overview = s.activity_review().unwrap();
    assert!(overview.blocks.is_empty());
    assert_eq!(overview.summaries.len(), 1);
    assert_eq!(overview.summaries[0].blocks[0].seconds, 3);
    assert_eq!(overview.summaries[0].blocks[0].status, "pending");
}
#[test]
fn pending_context_and_encrypted_images_survive_reopening() {
    let path = std::env::temp_dir().join(format!(
        "buddy-review-{}-{}.db",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let i = interval("2026-10-21T12:00:00Z", "ChatGPT", "ChatGPT");
    {
        let s = Storage::open(&path).unwrap();
        s.record_review_interval(&i, &unknown()).unwrap();
        s.retain_review_evidence(&i.activity, "image", "c3ludGhldGljLWpwZWc=")
            .unwrap();
    }
    {
        let s = Storage::open(&path).unwrap();
        let overview = s.activity_review().unwrap();
        assert_eq!(overview.pending_seconds, 3);
        let image = s.review_image(overview.blocks[0].evidence_ids[0]).unwrap();
        assert!(image.starts_with("data:image/jpeg;base64,"));
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn later_evidence_in_a_generic_window_does_not_assign_an_earlier_visit() {
    let mut s = fixture();
    let goal = add_goal(&mut s, "Prepare presentation");
    let morning = interval("2026-10-21T12:00:00Z", "ChatGPT", "ChatGPT");
    let later = interval("2026-10-21T12:20:00Z", "ChatGPT", "ChatGPT");
    s.record_review_interval(&morning, &unknown()).unwrap();
    s.record_review_interval(&later, &unknown()).unwrap();
    s.classify_review_context(&later.activity, Some(goal), None, "Preparing slides")
        .unwrap();
    let data = s.activity_review().unwrap();
    assert_eq!(data.pending_seconds, 3);
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 3);
    assert_eq!(
        data.blocks.iter().filter(|b| b.status == "pending").count(),
        1
    );
}

#[test]
fn retention_preserves_unfinished_review_but_explicit_clear_deletes_it() {
    let mut s = fixture();
    let goal = add_goal(&mut s, "Prepare presentation");
    s.transition_goal(goal, "resume").unwrap();
    let i = interval("2026-10-21T12:00:00Z", "ChatGPT", "ChatGPT");
    s.record_interval_with_match(goal, &i, Some(unknown()))
        .unwrap();
    s.record_review_interval(&i, &unknown()).unwrap();
    s.retain_review_evidence(&i.activity, "image", "c3ludGhldGljLWpwZWc=")
        .unwrap();
    s.connection.execute("INSERT INTO work_events(evidence_key,title,app,occurred_at,source,status) VALUES('retention-fixture','Reviewed release notes','ChatGPT',?1,'user_planned','planned')",[i.activity.timestamp.clone()]).unwrap();
    s.purge_history(Some("2026-10-30T00:00:00Z")).unwrap();
    let data = s.activity_review().unwrap();
    assert_eq!(data.pending_seconds, 3);
    assert_eq!(data.blocks[0].evidence_ids.len(), 1);
    let sources: i64 = s
        .connection
        .query_row("SELECT COUNT(*) FROM usage_intervals", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sources, 1);
    assert_eq!(
        s.connection
            .query_row(
                "SELECT COUNT(*) FROM work_events WHERE evidence_key='retention-fixture'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    s.purge_history(None).unwrap();
    assert!(s.activity_review().unwrap().blocks.is_empty());
    let evidence: i64 = s
        .connection
        .query_row("SELECT COUNT(*) FROM review_evidence", [], |r| r.get(0))
        .unwrap();
    assert_eq!(evidence, 0);
}

#[test]
fn transfer_splits_exact_measured_time_at_local_midnight() {
    let mut s = fixture();
    let goal = add_goal(&mut s, "Prepare presentation");
    s.transition_goal(goal, "resume").unwrap();
    let local = Local.with_ymd_and_hms(2026, 10, 21, 23, 59, 59).unwrap();
    let i = interval(&local.to_rfc3339(), "ChatGPT", "ChatGPT");
    s.record_interval_with_match(goal, &i, Some(unknown()))
        .unwrap();
    s.record_review_interval(&i, &unknown()).unwrap();
    let b = s.activity_review().unwrap().blocks.remove(0);
    s.review_action(b.period_id, &b.key, "assign", Some(goal), None)
        .unwrap();
    assert_eq!(s.relevant_seconds(goal, Some("2026-10-21")).unwrap(), 1);
    assert_eq!(s.relevant_seconds(goal, Some("2026-10-22")).unwrap(), 2);
    let b = s.activity_review().unwrap().blocks.remove(0);
    s.review_action(b.period_id, &b.key, "outside", None, None)
        .unwrap();
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 0);
}

#[test]
fn in_app_goal_planning_counts_without_a_legacy_active_goal_timer() {
    let mut s = fixture();
    let goal = add_goal(&mut s, "Prepare presentation");
    let i = interval(
        "2026-10-21T12:00:00Z",
        "desktop-buddy",
        "Planning goal in sumi",
    );
    s.record_review_interval(
        &i,
        &ActivityMatch {
            goal_id: Some(goal.to_string()),
            confidence: 1.,
            reason: "Explicit in-app planning context".into(),
        },
    )
    .unwrap();
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 3);
    assert!(s.core_timer().unwrap().is_none());
}

#[test]
fn retained_image_is_retried_for_its_original_segment_and_manual_review_updates_saved_summary() {
    let mut s = fixture();
    let goal = add_goal(&mut s, "Prepare presentation");
    let i = interval("2026-10-21T12:00:00Z", "ChatGPT", "ChatGPT");
    s.record_review_interval(&i, &unknown()).unwrap();
    s.retain_review_evidence(&i.activity, "image", "c3ludGhldGljLWpwZWc=")
        .unwrap();
    let b = s.activity_review().unwrap().blocks.remove(0);
    let (id, context, image) = s.unreviewed_image(b.period_id).unwrap().unwrap();
    assert_eq!(image, "c3ludGhldGljLWpwZWc=");
    assert_eq!(s.unreviewed_image(b.period_id).unwrap().unwrap().0, id);
    s.save_reviewed_image(b.period_id, id, &context, "Preparing presentation slides")
        .unwrap();
    assert!(s.unreviewed_image(b.period_id).unwrap().is_none());
    assert_eq!(
        s.activity_review().unwrap().blocks[0].description,
        "Preparing presentation slides"
    );
    s.connection
        .execute("UPDATE review_periods SET analysis_done=1", [])
        .unwrap();
    s.finish_due_reviews(Utc.with_ymd_and_hms(2026, 10, 23, 23, 0, 0).unwrap())
        .unwrap();
    assert!(s.review_image(id).is_err());
    let saved = s.activity_review().unwrap().summaries.remove(0);
    s.review_action(
        saved.period_id,
        &saved.blocks[0].key,
        "assign",
        Some(goal),
        None,
    )
    .unwrap();
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 3);
    assert_eq!(
        s.activity_review().unwrap().summaries[0].blocks[0].status,
        "assigned"
    );
}

#[test]
fn changing_summary_time_preserves_the_open_period_and_uses_new_time_next_period() {
    let s = fixture();
    let at = Local
        .with_ymd_and_hms(2026, 10, 21, 12, 0, 0)
        .unwrap()
        .with_timezone(&Utc);
    let first = s.review_period(at).unwrap();
    s.write_setting(
        "companion_preferences",
        &crate::companion::engine::Preferences {
            summary_minute: 20 * 60,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(s.review_period(at + Duration::hours(1)).unwrap(), first);
    let later = s.review_period(at + Duration::days(1)).unwrap();
    assert_ne!(first, later);
    let end: String = s
        .connection
        .query_row(
            "SELECT ends_at FROM review_periods WHERE id=?1",
            [later],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        DateTime::parse_from_rfc3339(&end)
            .unwrap()
            .with_timezone(&Local)
            .time(),
        chrono::NaiveTime::from_hms_opt(20, 0, 0).unwrap()
    );
}

#[test]
fn repeated_review_failures_close_the_period_with_pending_blocks_and_free_its_evidence() {
    let mut s = fixture();
    let now = Utc::now();
    let period = s.review_period(now).unwrap();
    for n in 1..MAX_REVIEW_FAILURES {
        assert!(!s
            .record_review_failure(period, "provider_timeout", now)
            .unwrap());
        let (failures, last): (i64, String) = s
            .connection
            .query_row(
                "SELECT failures,last_failure FROM review_periods WHERE id=?1",
                [period],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(failures, n);
        assert_eq!(last, "provider_timeout");
    }
    // The period has not ended, so even the last failure cannot finalise it yet.
    assert!(s
        .record_review_failure(period, "provider_timeout", now)
        .unwrap());
    let closed: bool = s
        .connection
        .query_row(
            "SELECT analysis_done=1 FROM review_periods WHERE id=?1",
            [period],
            |r| r.get(0),
        )
        .unwrap();
    assert!(closed);
    // Once the period's end has passed, the summary is saved and the evidence released.
    let later = now + Duration::days(2);
    s.finish_due_reviews(later).unwrap();
    let saved: bool = s
        .connection
        .query_row(
            "SELECT summary IS NOT NULL FROM review_periods WHERE id=?1",
            [period],
            |r| r.get(0),
        )
        .unwrap();
    assert!(saved);
    assert_eq!(s.review_vault_usage().unwrap().0, 0);
}
