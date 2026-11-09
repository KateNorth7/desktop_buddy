use super::*;
use crate::storage::Storage;

#[test]
fn passive_companion_stays_visible_while_quiet_but_never_displays_a_notification_card() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.buddy.view.preferences.suggestions_only = false;
    inner.buddy.view.suggestion = Some(Suggestion {
        id: 1,
        title: "Private".into(),
        url: "https://example.com".into(),
        reason: "Help".into(),
    });
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
    assert!(!inner.status.tracking);
    assert!(!inner.status.ai_enabled);
    assert!(
        !inner
            .storage
            .user_settings()
            .unwrap()
            .onboarding
            .tracking_consent
    );
    inner.status.tracking = true;
    inner.status.dnd = true;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
    inner.status.dnd = false;
    inner.buddy.view.quiet_reason = Some("Outside working hours".into());
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
    inner.buddy.view.preferences.suggestions_only = true;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 0);
}

#[test]
fn ratings_can_be_replaced_and_cleared_without_duplicates() {
    let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let goal = storage.set_goal("Learn Rust").unwrap();
    let id = storage
        .record_recommendation(goal.id, "Guide", "https://example.com", "Useful")
        .unwrap();
    for rating in [Some(true), Some(false), None] {
        storage.rate_recommendation(id, rating).unwrap();
        let history = storage.recommendation_history().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].feedback, rating);
    }
    assert!(storage.rate_recommendation(id + 1, Some(true)).is_err());
}

#[test]
fn quiet_mode_keeps_a_resting_mascot_without_pausing_tracking() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.status.tracking = true;
    inner.buddy.view.avatar.visible = true;
    inner.buddy.view.preferences.suggestions_only = false;
    inner.status.dnd = true;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
    assert!(inner.status.tracking);
    inner.status.dnd = false;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
}

#[test]
fn retired_research_never_schedules_while_display_respects_preferences_pause_and_dnd() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.status.tracking = true;
    crate::tracking::allow_notifications(&mut inner);
    inner.buddy.view.preferences.suggestions_only = true;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 0);
    inner.buddy.view.preferences.suggestions_only = false;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
    assert!(!due(&inner));
    inner.buddy.view.preferences.proactive = true;
    inner.status.mock_ai = false;
    inner.status.anthropic_configured = true;
    inner.status.web_search_configured = true;
    assert!(!due(&inner));
    inner.buddy.last_search = Some(Instant::now());
    assert!(!due(&inner));
    inner.buddy.last_search = Some(Instant::now() - Duration::from_secs(901));
    assert!(!due(&inner));
    inner.buddy.view.suggestion = Some(Suggestion {
        id: 1,
        title: "Resource".into(),
        url: "https://example.com".into(),
        reason: "Useful topic".into(),
    });
    assert_eq!(mode(&inner.status, &inner.buddy.view), 2);
    inner.buddy.view.avatar.visible = false;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 2);
    assert!(!due(&inner));
    inner.buddy.view.avatar.visible = true;
    inner.status.dnd = true;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
    assert!(!due(&inner));
    inner.status.dnd = false;
    inner.status.tracking = false;
    assert!(!due(&inner));
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
}
#[test]
fn persists_preferences_and_deduplicates_per_goal() {
    let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    assert!(!storage.buddy_preferences().unwrap().suggestions_only);
    assert!(!storage.buddy_preferences().unwrap().proactive);
    storage
        .save_buddy_preferences(&BuddyPreferences {
            suggestions_only: false,
            proactive: true,
            ..Default::default()
        })
        .unwrap();
    assert!(!storage.buddy_preferences().unwrap().suggestions_only);
    assert!(storage.buddy_preferences().unwrap().proactive);
    let goal = storage.set_goal("Learn Rust").unwrap();
    storage
        .save_suggestion(goal.id, "https://example.com/")
        .unwrap();
    storage
        .save_suggestion(goal.id, "https://example.com/")
        .unwrap();
    assert!(storage
        .seen_suggestion(goal.id, "https://example.com/")
        .unwrap());
    let next = storage.set_goal("Learn TypeScript").unwrap();
    assert!(!storage
        .seen_suggestion(next.id, "https://example.com/")
        .unwrap());
}

#[test]
fn single_fullscreen_setting_hides_both_mascot_and_chat() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.buddy.view.preferences.suggestions_only = false;
    inner.buddy.view.companion.preferences.hide_fullscreen = true;
    assert_eq!(display_mode(&inner.status, &inner.buddy.view, false), 1);
    assert_eq!(display_mode(&inner.status, &inner.buddy.view, true), 0);
    inner.buddy.view.companion.chat_open = true;
    assert_eq!(display_mode(&inner.status, &inner.buddy.view, true), 0);
    inner.buddy.view.companion.chat_open = false;
    inner.buddy.view.companion.preferences.hide_fullscreen = false;
    assert_eq!(display_mode(&inner.status, &inner.buddy.view, true), 1);
    inner.buddy.view.avatar.visible = false;
    assert_eq!(display_mode(&inner.status, &inner.buddy.view, false), 0);
}

#[test]
fn hidden_character_keeps_checkins_but_respects_quiet() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    let view = &mut inner.buddy.view;
    view.avatar.visible = false;
    view.companion.intervention = Some(crate::companion::engine::Intervention {
        id: "checkin".into(),
        kind: crate::companion::engine::Kind::Midday,
        text: "Next step?".into(),
        confidence: 1.0,
        goal_id: None,
        plan_revision: None,
        step_id: None,
        expires_at: i64::MAX,
    });
    assert_eq!(mode(&inner.status, &inner.buddy.view), 2);
    inner.status.dnd = true;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 0);
    inner.status.dnd = false;
    inner.buddy.view.quiet_reason = Some("Meeting".into());
    assert_eq!(mode(&inner.status, &inner.buddy.view), 0);
    inner.buddy.view.quiet_reason = None;
    inner.buddy.view.companion.intervention = None;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 0);
}

#[test]
fn mascot_anchor_survives_chat_resize_screen_edges_and_native_drag() {
    for scale in [1.0, 1.25, 2.0] {
        let size = ((160.0 * scale) as u32, (152.0 * scale) as u32);
        let anchor = BuddyPosition { x: 700, y: 740 };
        let fixed = clamp_position(&anchor, (0, 0), (1500, 900), size);
        for _ in 0..30 {
            for chat in [false, true, false] {
                let mascot = placement::Rect {
                    x: fixed.x,
                    y: fixed.y,
                    width: size.0,
                    height: size.1,
                };
                let work = placement::Rect {
                    x: 0,
                    y: 0,
                    width: 1500,
                    height: 900,
                };
                let _card = placement::place(work, mascot, scale, chat, 240, None);
                assert_eq!(clamp_position(&anchor, (0, 0), (1500, 900), size), fixed);
            }
        }
    }
}

#[test]
fn deadline_cards_continue_after_hours_but_respect_idle_meetings_and_quiet_mode() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.buddy.view.companion.intervention = Some(crate::companion::engine::Intervention {
        id: "deadline".into(),
        kind: crate::companion::engine::Kind::Deadline,
        text: "Task is overdue".into(),
        confidence: 1.0,
        goal_id: Some(1),
        plan_revision: None,
        step_id: None,
        expires_at: i64::MAX,
    });
    inner.status.tracking = false;
    inner.buddy.view.quiet_reason = None;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 2);
    for reason in ["You are away", "Meeting or presentation"] {
        inner.buddy.view.quiet_reason = Some(reason.into());
        assert_ne!(mode(&inner.status, &inner.buddy.view), 2);
    }
    inner
        .buddy
        .view
        .companion
        .intervention
        .as_mut()
        .unwrap()
        .kind = crate::companion::engine::Kind::EndOfDay;
    assert_ne!(mode(&inner.status, &inner.buddy.view), 2);
    inner.status.dnd = true;
    assert_ne!(mode(&inner.status, &inner.buddy.view), 2);
}

#[test]
fn quiet_mode_keeps_only_the_resting_mascot_even_if_chat_or_a_card_was_open() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.buddy.view.preferences.suggestions_only = false;
    inner.buddy.view.companion.chat_open = true;
    inner.status.dnd = true;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 1);
    inner.buddy.view.avatar.visible = false;
    assert_eq!(mode(&inner.status, &inner.buddy.view), 0);
}
