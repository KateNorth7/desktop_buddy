use super::*;
use crate::{
    browser,
    history::UsageInterval,
    insights::{Category, UsageTracker},
    models::ActivitySnapshot,
};
use chrono::{TimeZone, Utc};

fn snapshot(process: &str, title: &str) -> ActivitySnapshot {
    ActivitySnapshot {
        process_name: process.into(),
        window_title: title.into(),
        window_id: Some(1),
        browser: browser::context(process, title, None),
        ..Default::default()
    }
}
fn store() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}

#[test]
fn document_keys_separate_windows_that_share_a_title() {
    // A chat app or editor whose window title never changes is told apart by the
    // hashed document identity; the raw identity is never part of the snapshot.
    let mut a = snapshot("claude", "Claude");
    let mut b = a.clone();
    assert!(same_context(&a, &b));
    a.document_key = Some(crate::companion::engine::fingerprint("claude.ai/chat/one"));
    b.document_key = Some(crate::companion::engine::fingerprint("claude.ai/chat/two"));
    assert!(!same_context(&a, &b));
    b.document_key = a.document_key.clone();
    assert!(same_context(&a, &b));
    assert!(!serde_json::to_string(&a).unwrap().contains("claude.ai"));
    let mut tracker = UsageTracker::default();
    let start = Instant::now();
    assert!(tracker
        .observe_at(1, &a, true, ActivityState::Focused, start, Utc::now())
        .is_none());
    assert!(
        tracker
            .observe_at(
                1,
                &b,
                true,
                ActivityState::Focused,
                start + Duration::from_secs(3),
                Utc::now() + chrono::Duration::seconds(3)
            )
            .is_some(),
        "the same document continues the interval"
    );
    let mut c = b.clone();
    c.document_key = Some(crate::companion::engine::fingerprint(
        "claude.ai/chat/three",
    ));
    assert!(
        tracker
            .observe_at(
                1,
                &c,
                true,
                ActivityState::Focused,
                start + Duration::from_secs(6),
                Utc::now() + chrono::Duration::seconds(6)
            )
            .is_none(),
        "a different document starts a new interval"
    );
}

#[test]
fn transitions_between_apps_tabs_and_windows_do_not_bridge_contexts() {
    let now = Instant::now();
    let wall = Utc::now();
    let mut tracker = UsageTracker::default();
    let samples = [
        snapshot("editor.exe", "Code"),
        snapshot("chrome.exe", "Tab A"),
        snapshot("chrome.exe", "Tab B"),
        snapshot("editor.exe", "Code"),
    ];
    for (index, a) in samples.iter().enumerate() {
        let t = index as u64 * 6;
        assert!(tracker
            .observe_at(
                1,
                a,
                true,
                ActivityState::Focused,
                now + Duration::from_secs(t),
                wall + chrono::Duration::seconds(t as i64)
            )
            .is_none());
        let interval = tracker
            .observe_at(
                1,
                a,
                true,
                ActivityState::Focused,
                now + Duration::from_secs(t + 3),
                wall + chrono::Duration::seconds(t as i64 + 3),
            )
            .unwrap();
        assert_eq!((interval.end - interval.start).num_seconds(), 3);
        assert_eq!(interval.activity.window_title, a.window_title);
    }
    let mut changed = samples[3].clone();
    changed.window_id = Some(2);
    assert!(tracker
        .observe_at(
            1,
            &changed,
            true,
            ActivityState::Focused,
            now + Duration::from_secs(24),
            wall + chrono::Duration::seconds(24)
        )
        .is_none());
}

#[test]
fn identical_titles_on_different_sites_are_distinct_and_legacy_metadata_is_optional() {
    let mut a = snapshot("msedge.exe", "Dashboard");
    let mut b = a.clone();
    a.browser.as_mut().unwrap().domain = Some("a.example.com".into());
    b.browser.as_mut().unwrap().domain = Some("b.example.com".into());
    assert!(!same_context(&a, &b));
    let legacy: ActivitySnapshot=serde_json::from_str(r#"{"timestamp":"old","process_name":"chrome.exe","window_title":"Old","idle_seconds":0,"active_seconds":7}"#).unwrap();
    assert!(legacy.browser.is_none());
    assert!(legacy.window_id.is_none());
}

#[test]
fn state_detection_is_explainable_and_resumes_after_idle_or_drifting() {
    let mut tracker = StateTracker::default();
    let settings = TrackingSettings::default();
    let now = Instant::now();
    let mut a = snapshot("chrome.exe", "Documentation");
    assert_eq!(
        tracker.sample_at(1, &a, true, &settings, now),
        ActivityState::Focused
    );
    assert_eq!(tracker.event, ActivityEvent::Working);
    for t in (3..=180).step_by(3) {
        a.idle_seconds = t;
        tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(t));
    }
    assert_eq!(tracker.state, ActivityState::Drifting);
    assert_eq!(tracker.event, ActivityEvent::Drifting);
    let revision = tracker.revision;
    a.idle_seconds = 183;
    tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(183));
    assert_eq!(tracker.revision, revision);
    a.idle_seconds = 0;
    assert_eq!(
        tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(186)),
        ActivityState::Focused
    );
    assert_eq!(tracker.event, ActivityEvent::Resumed);
    a.idle_seconds = 300;
    assert_eq!(
        tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(189)),
        ActivityState::Paused
    );
    a.idle_seconds = 0;
    assert_eq!(
        tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(192)),
        ActivityState::Focused
    );
    assert_eq!(tracker.event, ActivityEvent::Resumed);
    tracker.stop(true);
    assert_eq!(tracker.event, ActivityEvent::GoalCompleted);
}

#[test]
fn media_and_frequent_input_do_not_trigger_drifting() {
    let settings = TrackingSettings::default();
    let now = Instant::now();
    for media in [true, false] {
        let mut tracker = StateTracker::default();
        let mut a = snapshot("chrome.exe", "Video or document");
        a.media_playing = media;
        for t in (0..=210).step_by(3) {
            a.idle_seconds = if media { t } else { 0 };
            assert_eq!(
                tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(t)),
                ActivityState::Focused
            );
        }
    }
}

#[test]
fn context_changes_gaps_and_exclusions_reset_drifting_timer() {
    let now = Instant::now();
    let mut tracker = StateTracker::default();
    let settings = TrackingSettings::default();
    let mut a = snapshot("chrome.exe", "A");
    a.idle_seconds = 180;
    tracker.sample_at(1, &a, true, &settings, now);
    assert_eq!(
        tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(180)),
        ActivityState::Focused
    );
    a.window_title = "B".into();
    assert_eq!(
        tracker.sample_at(1, &a, true, &settings, now + Duration::from_secs(183)),
        ActivityState::Focused
    );
    assert_eq!(
        tracker.sample_at(1, &a, false, &settings, now + Duration::from_secs(186)),
        ActivityState::Paused
    );
}

#[test]
fn intervals_skip_idle_transitions_sleep_restart_clock_and_goal_changes() {
    let now = Instant::now();
    let wall = Utc::now();
    let a = snapshot("editor.exe", "Code");
    let mut t = UsageTracker::default();
    assert!(t
        .observe_at(1, &a, true, ActivityState::Focused, now, wall)
        .is_none());
    assert!(t
        .observe_at(
            1,
            &a,
            true,
            ActivityState::Paused,
            now + Duration::from_secs(3),
            wall + chrono::Duration::seconds(3)
        )
        .is_none());
    let idle = t
        .observe_at(
            1,
            &a,
            true,
            ActivityState::Paused,
            now + Duration::from_secs(6),
            wall + chrono::Duration::seconds(6),
        )
        .unwrap();
    assert_eq!(idle.state, ActivityState::Paused);
    assert!(t
        .observe_at(
            1,
            &a,
            true,
            ActivityState::Focused,
            now + Duration::from_secs(9),
            wall + chrono::Duration::seconds(9)
        )
        .is_none());
    assert!(t
        .observe_at(
            1,
            &a,
            true,
            ActivityState::Focused,
            now + Duration::from_secs(100),
            wall + chrono::Duration::seconds(100)
        )
        .is_none());
    assert!(t
        .observe_at(
            1,
            &a,
            true,
            ActivityState::Focused,
            now + Duration::from_secs(103),
            wall + chrono::Duration::hours(1)
        )
        .is_none());
    assert!(t
        .observe_at(
            2,
            &a,
            true,
            ActivityState::Focused,
            now + Duration::from_secs(106),
            wall + chrono::Duration::hours(1) + chrono::Duration::seconds(3)
        )
        .is_none());
    assert!(UsageTracker::default()
        .observe_at(2, &a, true, ActivityState::Focused, now, wall)
        .is_none());
}

#[test]
fn settings_validate_persist_and_obey_day_and_overnight_boundaries() {
    let s = store();
    let mut p = TrackingSettings::default();
    for (minute, allowed) in [(539, false), (540, true), (1079, true), (1080, false)] {
        assert_eq!(p.working_at(minute), allowed);
    }
    p.working_start_minute = 1320;
    p.working_end_minute = 360;
    assert!(p.working_at(1439));
    assert!(p.working_at(0));
    assert!(!p.working_at(360));
    p.idle_seconds = 600;
    s.write_setting("tracking_settings", &p).unwrap();
    assert_eq!(s.tracking_settings().unwrap().idle_seconds, 600);
    p.working_start_minute = 1440;
    assert!(p.validate().is_err());
}

#[test]
fn activity_delivery_ignores_working_hours_but_respects_quiet_idle_and_media() {
    let state = crate::commands::AppState::new(store()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    let g = inner.storage.set_goal("Goal").unwrap();
    inner.status.tracking = true;
    inner.status.ai_enabled = true;
    inner.status.anthropic_configured = true;
    inner.status.web_search_configured = true;
    inner.status.mock_ai = false;
    inner.buddy.view.preferences.proactive = true;
    inner.buddy.view.preferences.local_nudges = true;
    let a = snapshot("editor.exe", "Code");
    inner.buddy.foreground = Some(a);
    allow_notifications(&mut inner);
    assert!(!crate::buddy::due(&inner));
    let start = inner.tracking_settings.working_end_minute;
    inner.tracking_settings.working_start_minute = start;
    inner.tracking_settings.working_end_minute = (start + 60) % 1440;
    assert!(!crate::buddy::due(&inner));
    assert!(notifications_allowed(&inner));
    inner.status.dnd = true;
    assert!(!notifications_allowed(&inner));
    inner.status.dnd = false;
    crate::insights::local_nudge(&mut inner).unwrap();
    assert!(inner.buddy.shown.is_none());
    drop(inner);
    assert!(!crate::commands::should_analyze(&state));
    let mut inner = state.inner.lock().unwrap();
    allow_notifications(&mut inner);
    inner.buddy.foreground.as_mut().unwrap().media_playing = true;
    assert!(!crate::buddy::due(&inner));
    inner.buddy.foreground.as_mut().unwrap().media_playing = false;
    inner.activity_state.stop(false);
    assert!(!notifications_allowed(&inner));
    assert_eq!(inner.storage.goal().unwrap().unwrap().id, g.id);
}

#[test]
fn history_is_disjoint_goal_specific_and_retains_recommendations_after_restart() {
    let path = std::env::temp_dir().join(format!(
        "buddy-tracking-{}-{}.db",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let goal;
    {
        let mut s = Storage::open(&path).unwrap();
        goal = s.set_goal("Learn Rust").unwrap();
        s.save_app_rule(goal.id, "chrome.exe", Some(Category::Work))
            .unwrap();
        let start = Utc.with_ymd_and_hms(2026, 10, 2, 9, 0, 0).unwrap();
        let mut a = snapshot("chrome.exe", "Rust docs");
        a.browser.as_mut().unwrap().domain = Some("doc.rust-lang.org".into());
        for (i, state) in [
            ActivityState::Focused,
            ActivityState::Paused,
            ActivityState::Drifting,
        ]
        .iter()
        .enumerate()
        {
            let interval = UsageInterval {
                start: start + chrono::Duration::seconds(i as i64 * 3),
                end: start + chrono::Duration::seconds(i as i64 * 3 + 3),
                activity: a.clone(),
                state: *state,
            };
            s.record_interval(goal.id, &interval).unwrap();
            assert!(s.record_interval(goal.id, &interval).is_err());
        }
        s.tracking_event(goal.id, ActivityEvent::Drifting).unwrap();
        s.tracking_event(goal.id, ActivityEvent::Paused).unwrap();
        s.record_recommendation(goal.id, "Guide", "https://example.com", "Useful")
            .unwrap();
        s.transition_goal(goal.id, "complete").unwrap();
        s.set_goal("Another goal").unwrap();
    }
    {
        let mut s = Storage::open(&path).unwrap();
        let records = s.goal_history(None).unwrap();
        assert_eq!(records.len(), 1);
        let h = &records[0];
        assert_eq!(h.goal.id, goal.id);
        assert!(h.completed_at.is_some());
        assert!(h.elapsed_milliseconds.is_some());
        assert_eq!(h.active_milliseconds, 6000);
        assert_eq!(h.focused_milliseconds, 3000);
        assert_eq!(h.idle_milliseconds, 3000);
        assert_eq!(h.drifting_milliseconds, 3000);
        assert_eq!(h.drifting_events, 1);
        assert_eq!(h.pause_events, 1);
        assert_eq!(h.applications[0].active_milliseconds, 6000);
        assert_eq!(h.sites[0].domain.as_deref(), Some("doc.rust-lang.org"));
        assert_eq!(h.tabs.len(), 1);
        assert_eq!(h.recommendations.len(), 1);
        let segments = s.activity_segments(Some(goal.id)).unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].goal_id, goal.id.to_string());
        assert_eq!(segments[0].app, "chrome.exe");
        assert_eq!(segments[0].title.as_deref(), Some("Rust docs"));
        assert_eq!(segments[0].domain.as_deref(), Some("doc.rust-lang.org"));
        assert!(segments[0].url.is_none());
        assert_eq!(segments[0].duration_seconds, 3.0);
        assert_eq!(segments[0].state, "drifting");
        let interval = UsageInterval {
            start: Utc::now(),
            end: Utc::now() + chrono::Duration::seconds(3),
            activity: snapshot("editor.exe", "Code"),
            state: ActivityState::Focused,
        };
        assert!(s.record_interval(goal.id, &interval).is_err());
        s.purge_history(None).unwrap();
        assert!(s.goal_history(Some(goal.id)).unwrap().is_empty());
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_schema_migration_keeps_activity_and_unknown_completion_times() {
    let path = std::env::temp_dir().join(format!(
        "buddy-legacy-{}-{}.db",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE goals(id INTEGER PRIMARY KEY,text TEXT,status TEXT,created_at TEXT);CREATE TABLE activity(id INTEGER PRIMARY KEY,goal_id INTEGER,timestamp TEXT,process_name TEXT,window_title TEXT,idle_seconds INTEGER,active_seconds INTEGER);INSERT INTO goals VALUES(1,'Legacy','completed','2026-01-01T00:00:00Z');INSERT INTO activity VALUES(1,1,'2026-01-01T00:00:00Z','chrome.exe','Old',0,900);").unwrap();
    }
    {
        let s = Storage::open(&path).unwrap();
        s.initialize_tracking().unwrap();
        let a = s.recent(1).unwrap();
        assert_eq!(a[0].active_seconds, 900);
        assert!(a[0].browser.is_none());
        let h = s.goal_history(None).unwrap();
        assert_eq!(h[0].active_milliseconds, 0);
        assert!(h[0].completed_at.is_none());
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn previous_daily_totals_are_preserved_without_inventing_sites_or_focus() {
    let mut s = store();
    let g = s.set_goal("Previously observed").unwrap();
    let start = Utc::now();
    s.record_usage(
        g.id,
        "editor.exe",
        start,
        start + chrono::Duration::seconds(6),
    )
    .unwrap();
    s.transition_goal(g.id, "complete").unwrap();
    let h = s.goal_history(None).unwrap();
    assert_eq!(h[0].active_milliseconds, 6000);
    assert_eq!(h[0].unattributed_active_milliseconds, 6000);
    assert_eq!(h[0].applications[0].active_milliseconds, 6000);
    assert_eq!(h[0].focused_milliseconds, 0);
    assert!(h[0].sites.is_empty());
}

#[test]
fn local_drifting_reminder_requires_opt_in_and_suppresses_confirmed_media() {
    let state = crate::commands::AppState::new(store()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.storage.set_goal("Goal").unwrap();
    inner.status.tracking = true;
    allow_notifications(&mut inner);
    inner.activity_state.state = ActivityState::Drifting;
    let mut a = snapshot("editor.exe", "Reading");
    a.idle_seconds = 180;
    inner.buddy.foreground = Some(a);
    crate::insights::local_nudge(&mut inner).unwrap();
    assert!(inner.buddy.shown.is_none());
    inner.buddy.view.preferences.local_nudges = true;
    inner.buddy.foreground.as_mut().unwrap().media_playing = true;
    crate::insights::local_nudge(&mut inner).unwrap();
    assert!(inner.buddy.shown.is_none());
    inner.buddy.foreground.as_mut().unwrap().media_playing = false;
    crate::insights::local_nudge(&mut inner).unwrap();
    assert!(inner.buddy.shown.is_none());
    assert!(inner.buddy.view.decision.is_none());
}

#[test]
fn one_goal_monitoring_switch_persists_time_and_progress_consent() {
    let state =
        crate::commands::AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap())
            .unwrap();
    let mut inner = state.inner.lock().unwrap();
    assert!(configure_goal_monitoring(&mut inner, true).is_err());
    let mut settings = inner.storage.user_settings().unwrap();
    settings.onboarding.completed = true;
    inner
        .storage
        .write_setting("user_settings", &settings)
        .unwrap();
    configure_goal_monitoring(&mut inner, true).unwrap();
    assert!(requested(&inner.storage).unwrap());
    assert!(inner.companion.view.preferences.screen_task_detection);
    assert!(inner.storage.ai_preferences().unwrap().enabled);
    assert!(
        crate::companion::Runtime::load(&inner.storage)
            .unwrap()
            .view
            .preferences
            .screen_task_detection
    );
    configure_goal_monitoring(&mut inner, false).unwrap();
    assert!(!requested(&inner.storage).unwrap());
    assert!(!inner.companion.view.preferences.screen_task_detection);
    assert!(inner.storage.ai_preferences().unwrap().enabled);
}

#[test]
fn goal_monitoring_choice_survives_database_reopen_and_restart() {
    let path = std::env::temp_dir().join(format!(
        "buddy-monitoring-restart-{}-{}.db",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    {
        let state = crate::commands::AppState::new(Storage::open(&path).unwrap()).unwrap();
        let mut inner = state.inner.lock().unwrap();
        let mut settings = inner.storage.user_settings().unwrap();
        settings.onboarding.completed = true;
        inner
            .storage
            .write_setting("user_settings", &settings)
            .unwrap();
        configure_goal_monitoring(&mut inner, true).unwrap();
    }
    {
        let state = crate::commands::AppState::new(Storage::open(&path).unwrap()).unwrap();
        let mut inner = state.inner.lock().unwrap();
        assert!(inner.status.tracking);
        assert!(requested(&inner.storage).unwrap());
        assert!(inner.companion.view.preferences.screen_task_detection);
        configure_goal_monitoring(&mut inner, false).unwrap();
    }
    {
        let state = crate::commands::AppState::new(Storage::open(&path).unwrap()).unwrap();
        let inner = state.inner.lock().unwrap();
        assert!(!inner.status.tracking);
        assert!(!requested(&inner.storage).unwrap());
        assert!(!inner.companion.view.preferences.screen_task_detection);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn passive_foreground_playback_is_not_idle_but_disabled_tracking_still_wins() {
    let mut state = StateTracker::default();
    let mut activity = snapshot("Future media application", "An arbitrary task");
    activity.idle_seconds = 1800;
    activity.media_playing = true;
    let settings = TrackingSettings::default();
    assert_eq!(
        state.sample(1, &activity, true, &settings),
        ActivityState::Focused
    );
    activity.media_playing = false;
    assert_eq!(
        state.sample(1, &activity, true, &settings),
        ActivityState::Paused
    );
    activity.media_playing = true;
    assert_eq!(
        state.sample(1, &activity, false, &settings),
        ActivityState::Paused
    );
}
