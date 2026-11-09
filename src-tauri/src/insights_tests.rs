use super::*;
use chrono::{Duration, TimeZone};

fn store() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}
fn snapshot() -> ActivitySnapshot {
    ActivitySnapshot {
        timestamp: String::new(),
        process_name: "editor.exe".into(),
        window_title: "Document".into(),
        idle_seconds: 5,
        active_seconds: 180,
        ..Default::default()
    }
}

#[test]
fn rules_are_goal_specific_normalized_removable_and_validated() {
    let mut s = store();
    let first = s.set_goal("First").unwrap();
    s.save_app_rule(first.id, " Editor.EXE ", Some(Category::Work))
        .unwrap();
    assert_eq!(
        s.category(first.id, "EDITOR.EXE").unwrap(),
        Some(Category::Work)
    );
    assert!(s
        .save_app_rule(first.id, "C:\\editor.exe", Some(Category::Work))
        .is_err());
    let second = s.set_goal("Second").unwrap();
    assert!(s.category(second.id, "editor.exe").unwrap().is_none());
    assert!(s.save_app_rule(first.id, "editor.exe", None).is_err());
    s.save_app_rule(second.id, "editor.exe", Some(Category::Neutral))
        .unwrap();
    s.save_app_rule(second.id, "editor.exe", None).unwrap();
    assert!(s.app_rules(second.id).unwrap().is_empty());
}

#[test]
fn daily_usage_splits_midnight_and_aggregates_goals_without_legacy_backfill() {
    let mut s = store();
    let first = s.set_goal("First").unwrap();
    s.activity(first.id, &snapshot()).unwrap();
    assert!(s.today(Some(first.id)).unwrap().apps.is_empty());
    let midnight = Local
        .with_ymd_and_hms(2026, 9, 20, 0, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    s.record_usage(
        first.id,
        "editor.exe",
        midnight - Duration::seconds(2),
        midnight + Duration::seconds(3),
    )
    .unwrap();
    let amounts: Vec<(String, i64)> = s
        .connection
        .prepare("SELECT day,milliseconds FROM usage_daily ORDER BY day")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        amounts,
        vec![("2026-09-19".into(), 2000), ("2026-09-20".into(), 3000)]
    );
    // Isolate the midnight fixture from today's aggregation, even on that date.
    s.connection.execute("DELETE FROM usage_daily", []).unwrap();
    let now = Local::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    s.record_usage(first.id, "editor.exe", now, now + Duration::seconds(3))
        .unwrap();
    let second = s.set_goal("Second").unwrap();
    s.record_usage(second.id, "editor.exe", now, now + Duration::seconds(6))
        .unwrap();
    let today = s.today(Some(second.id)).unwrap();
    assert_eq!(today.apps[0].seconds, 9);
    assert_eq!(today.apps[0].goal_seconds, 6);
    s.record_usage(second.id, "editor.exe", now, now + Duration::seconds(16))
        .unwrap();
    assert_eq!(s.today(Some(second.id)).unwrap().apps[0].seconds, 9);
}

#[test]
fn retention_clears_usage_but_preserves_active_rules_and_budget() {
    let mut s = store();
    let g = s.set_goal("Goal").unwrap();
    s.save_app_rule(g.id, "editor.exe", Some(Category::Work))
        .unwrap();
    let now = Utc::now();
    s.record_usage(g.id, "editor.exe", now, now + Duration::seconds(3))
        .unwrap();
    s.record_usage(
        g.id,
        "old.exe",
        now - Duration::days(10),
        now - Duration::days(10) + Duration::seconds(3),
    )
    .unwrap();
    s.purge_history(Some(&(now - Duration::days(7)).to_rfc3339()))
        .unwrap();
    let count: i64 = s
        .connection
        .query_row("SELECT COUNT(*) FROM usage_daily", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert!(s.reserve_nudge(&BuddyPreferences::default()).unwrap());
    s.purge_history(None).unwrap();
    assert!(s.today(Some(g.id)).unwrap().apps.is_empty());
    assert_eq!(s.app_rules(g.id).unwrap().len(), 1);
    assert!(!s.nudge_allowed(&BuddyPreferences::default()).unwrap());
}

#[test]
fn budget_obeys_daily_limit_cooldown_day_rollover_and_zero() {
    let mut p = BuddyPreferences {
        daily_nudge_limit: 3,
        ..BuddyPreferences::default()
    };
    let mut b = NudgeBudget {
        day: "2026-09-30".into(),
        count: 3,
        last_at: Some(1000),
    };
    assert!(!b.permits(&p, "2026-09-30", 10000));
    assert!(!b.permits(&p, "2026-10-01", 1100));
    assert!(b.permits(&p, "2026-10-01", 1900));
    b.count = 2;
    assert!(!b.permits(&p, "2026-09-30", 999));
    assert!(b.permits(&p, "2026-09-30", 1900));
    p.daily_nudge_limit = 0;
    assert!(!b.permits(&p, "2026-10-01", 10000));
    p.daily_nudge_limit = 51;
    assert!(crate::attention::validate_preferences(p).is_err());
}

#[test]
fn local_rules_need_opt_in_and_respect_all_quiet_controls_without_ai() {
    let state = AppState::new(store()).unwrap();
    let mut i = state.inner.lock().unwrap();
    let goal = i.storage.set_goal("Goal").unwrap();
    i.status.tracking = true;
    crate::tracking::allow_notifications(&mut i);
    i.status.ai_enabled = false;
    i.buddy.foreground = Some(snapshot());
    i.storage
        .save_app_rule(goal.id, "editor.exe", Some(Category::Distraction))
        .unwrap();
    local_nudge(&mut i).unwrap();
    assert!(i.buddy.shown.is_none());
    i.buddy.view.preferences.local_nudges = true;
    i.status.dnd = true;
    local_nudge(&mut i).unwrap();
    assert!(i.buddy.shown.is_none());
    i.status.dnd = false;
    i.buddy.fullscreen = true;
    local_nudge(&mut i).unwrap();
    assert!(i.buddy.shown.is_none());
    i.buddy.fullscreen = false;
    i.status.dnd = true;
    local_nudge(&mut i).unwrap();
    assert!(i.buddy.shown.is_none());
    i.status.dnd = false;
    i.buddy.foreground.as_mut().unwrap().active_seconds = 119;
    local_nudge(&mut i).unwrap();
    assert!(i.buddy.shown.is_none());
    i.buddy.foreground.as_mut().unwrap().active_seconds = 120;
    for category in [Category::Work, Category::Neutral] {
        i.storage
            .save_app_rule(goal.id, "editor.exe", Some(category))
            .unwrap();
        local_nudge(&mut i).unwrap();
        assert!(i.buddy.shown.is_none());
    }
    i.storage
        .save_app_rule(goal.id, "editor.exe", Some(Category::Distraction))
        .unwrap();
    local_nudge(&mut i).unwrap();
    assert!(i.buddy.view.decision.is_none());
    assert_eq!(explicit_category(&i).unwrap(), Some(Category::Distraction));
    assert!(!i.status.ai_enabled);
}

#[test]
fn additive_schema_and_preferences_preserve_legacy_data() {
    let mut s = store();
    let goal = s.set_goal("Existing").unwrap();
    s.initialize().unwrap();
    assert_eq!(s.goal().unwrap().unwrap().id, goal.id);
    let p: BuddyPreferences = serde_json::from_str(r#"{"proactive":true}"#).unwrap();
    assert_eq!(p.daily_nudge_limit, 8);
    assert!(!p.local_nudges);
    assert!(p.proactive);
}

#[test]
fn explicit_rules_skip_automatic_ai_and_budget_blocks_searches() {
    let state = AppState::new(store()).unwrap();
    let mut i = state.inner.lock().unwrap();
    let g = i.storage.set_goal("Goal").unwrap();
    i.status.tracking = true;
    crate::tracking::allow_notifications(&mut i);
    i.status.ai_enabled = true;
    i.status.mock_ai = false;
    i.status.anthropic_configured = true;
    i.status.web_search_configured = true;
    i.buddy.foreground = Some(snapshot());
    i.buddy.view.preferences.proactive = true;
    drop(i);
    assert!(!crate::commands::should_analyze(&state));
    for category in [Category::Work, Category::Neutral, Category::Distraction] {
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .save_app_rule(g.id, "editor.exe", Some(category))
            .unwrap();
        assert!(!crate::commands::should_analyze(&state));
    }
    let i = state.inner.lock().unwrap();
    i.storage.save_app_rule(g.id, "editor.exe", None).unwrap();
    assert!(!crate::buddy::due(&i));
    i.storage.reserve_nudge(&i.buddy.view.preferences).unwrap();
    assert!(!crate::buddy::due(&i));
    drop(i);
    assert!(!crate::commands::should_analyze(&state));
}

#[test]
fn rules_usage_and_budget_survive_database_reopening() {
    let path = std::env::temp_dir().join(format!(
        "buddy-insights-{}-{}.db",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let id;
    {
        let mut s = Storage::open(&path).unwrap();
        id = s.set_goal("Persist").unwrap().id;
        s.save_app_rule(id, "editor.exe", Some(Category::Work))
            .unwrap();
        let now = Utc::now();
        s.record_usage(id, "editor.exe", now, now + Duration::seconds(3))
            .unwrap();
        assert!(s.reserve_nudge(&BuddyPreferences::default()).unwrap());
    }
    {
        let s = Storage::open(&path).unwrap();
        assert_eq!(s.category(id, "editor.exe").unwrap(), Some(Category::Work));
        let ms: i64 = s
            .connection
            .query_row("SELECT SUM(milliseconds) FROM usage_daily", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ms, 3000);
        assert!(!s.nudge_allowed(&BuddyPreferences::default()).unwrap());
    }
    std::fs::remove_file(path).unwrap();
}
