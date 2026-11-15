use super::*;
use crate::storage::Storage;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
fn fixture() -> (AppState, ActivitySnapshot) {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let activity = ActivitySnapshot {
        process_name: "Editor".into(),
        window_title: "Release notes".into(),
        window_id: Some(42),
        ..Default::default()
    };
    {
        let mut inner = state.inner.lock().unwrap();
        inner.status.tracking = true;
        inner.status.mock_ai = false;
        inner.status.demo = false;
        inner.companion.view.preferences.screen_task_detection = true;
        inner.buddy.foreground = Some(activity.clone());
    }
    (state, activity)
}

#[tokio::test]
async fn browser_metadata_gap_discards_cached_text_and_rejects_an_inflight_read_after_return() {
    for in_flight in [false, true] {
        let (state, mut activity) = fixture();
        activity.process_name = "chrome.exe".into();
        activity.browser = crate::browser::context(
            &activity.process_name,
            &activity.window_title,
            Some("example.com".into()),
        );
        let mut unknown = activity.clone();
        unknown.browser =
            crate::browser::context(&activity.process_name, &activity.window_title, None);
        {
            let mut inner = state.inner.lock().unwrap();
            crate::goal_matching::observe_inner(&mut inner, &activity, true).unwrap();
            inner.buddy.foreground = Some(activity.clone());
        }
        let change = || {
            let mut inner = state.inner.lock().unwrap();
            crate::goal_matching::observe_inner(&mut inner, &unknown, true).unwrap();
            inner.buddy.foreground = Some(unknown.clone());
            assert!(inner.companion.screen_context.is_none());
            crate::goal_matching::observe_inner(&mut inner, &activity, true).unwrap();
            inner.buddy.foreground = Some(activity.clone());
        };
        let old = read_at(&state, &activity, Instant::now(), |_| async {
            if in_flight {
                change();
            }
            Ok("Old synthetic page evidence".into())
        })
        .await
        .unwrap();
        if in_flight {
            assert!(old.is_none());
        } else {
            assert!(old.is_some());
            change();
        }
        let fresh = read_at(&state, &activity, Instant::now(), |_| async {
            Ok("Fresh synthetic page evidence".into())
        })
        .await
        .unwrap();
        assert_eq!(fresh.as_deref(), Some("Fresh synthetic page evidence"));
    }
}

#[tokio::test]
async fn matching_and_completion_share_one_redacted_read_until_expiry_or_window_change() {
    let (state, activity) = fixture();
    let reads = AtomicUsize::new(0);
    let base = Instant::now();
    let first = read_at(&state, &activity, base, |_| async {
        reads.fetch_add(1, Ordering::SeqCst);
        Ok("Release notes\nAPI_KEY=fictional-private-canary\nReview the change".into())
    })
    .await
    .unwrap()
    .unwrap();
    // read_at accounts for lock acquisition time; measure expiry from the stored timestamp.
    let cached_at = state
        .inner
        .lock()
        .unwrap()
        .companion
        .screen_context
        .as_ref()
        .unwrap()
        .at;
    // Changing a goal/checklist must not cause the second consumer to read again.
    state.inner.lock().unwrap().companion.invalidate();
    let cached = read_at(
        &state,
        &activity,
        cached_at + Duration::from_secs(19),
        |_| async { panic!("A second Accessibility read is forbidden") },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first, cached);
    assert!(!cached.contains("API_KEY"));
    assert!(!cached.contains("fictional-private-canary"));
    assert!(cached.contains("[sensitive line omitted]"));
    assert!(matching_excerpt(&activity, true, Some(&cached))
        .unwrap()
        .contains("Release notes"));
    let fresh = read_at(
        &state,
        &activity,
        cached_at + Duration::from_secs(20),
        |_| async {
            reads.fetch_add(1, Ordering::SeqCst);
            Ok("Fresh screen".into())
        },
    )
    .await
    .unwrap();
    assert_eq!(fresh.as_deref(), Some("Fresh screen"));
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    let changed = ActivitySnapshot {
        window_id: Some(43),
        ..activity
    };
    state.inner.lock().unwrap().buddy.foreground = Some(changed.clone());
    read_at(
        &state,
        &changed,
        cached_at + Duration::from_secs(21),
        |_| async {
            reads.fetch_add(1, Ordering::SeqCst);
            Ok("Other window".into())
        },
    )
    .await
    .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 3);
}
#[tokio::test]
async fn concurrent_consumers_and_unavailable_samples_never_duplicate_the_accessibility_read() {
    let (state, activity) = fixture();
    let reads = AtomicUsize::new(0);
    let reader = |_| async {
        reads.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok("Shared screen".into())
    };
    let (a, b) = tokio::join!(
        read_at(&state, &activity, Instant::now(), reader),
        read_at(&state, &activity, Instant::now(), reader)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    state.inner.lock().unwrap().mark_consent_change();
    let base = Instant::now();
    assert!(read_at(&state, &activity, base, |_| async {
        reads.fetch_add(1, Ordering::SeqCst);
        Err("Screen context unavailable".into())
    })
    .await
    .is_err());
    assert!(read_at(
        &state,
        &activity,
        base + Duration::from_secs(1),
        |_| async { panic!("Unavailable reads must also be cached") }
    )
    .await
    .is_err());
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn opt_out_sensitive_apps_missing_identity_and_changed_foreground_never_read_or_reuse_text() {
    for condition in [
        "off",
        "password_manager",
        "no_window",
        "different_window",
        "excluded_app",
    ] {
        let (state, mut activity) = fixture();
        read_at(&state, &activity, Instant::now(), |_| async {
            Ok("Cached text".into())
        })
        .await
        .unwrap();
        {
            let mut inner = state.inner.lock().unwrap();
            match condition {
                "off" => inner.companion.view.preferences.screen_task_detection = false,
                "password_manager" => {
                    activity.process_name = "1Password".into();
                    inner.buddy.foreground = Some(activity.clone());
                }
                "no_window" => {
                    activity.window_id = None;
                    inner.buddy.foreground = Some(activity.clone());
                }
                "different_window" => inner.buddy.foreground.as_mut().unwrap().window_id = Some(99),
                "excluded_app" => inner
                    .buddy
                    .view
                    .preferences
                    .excluded_apps
                    .push("Editor".into()),
                _ => unreachable!(),
            }
        }
        assert_eq!(
            read_at(&state, &activity, Instant::now(), |_| async {
                panic!("Forbidden Accessibility read")
            })
            .await
            .unwrap(),
            None,
            "{condition}"
        );
        assert!(state
            .inner
            .lock()
            .unwrap()
            .companion
            .screen_context
            .is_none());
    }
}
#[tokio::test]
async fn consent_changes_and_window_switches_during_sampling_discard_the_result() {
    for condition in ["consent", "off_then_on", "window"] {
        let (state, activity) = fixture();
        let result = read_at(&state, &activity, Instant::now(), |_| async {
            let mut inner = state.inner.lock().unwrap();
            match condition {
                "consent" => inner.mark_consent_change(),
                "off_then_on" => {
                    inner.companion.invalidate_screen_context();
                    inner.privacy_revision += 2;
                }
                "window" => inner.buddy.foreground.as_mut().unwrap().window_id = Some(99),
                _ => unreachable!(),
            }
            Ok("Discard this screen".into())
        })
        .await
        .unwrap();
        assert!(result.is_none());
        assert!(state
            .inner
            .lock()
            .unwrap()
            .companion
            .screen_context
            .is_none());
    }
}
#[test]
fn excerpts_redact_whole_secret_lines_before_the_character_limit() {
    let (_, activity) = fixture();
    let raw = format!(
        "{}api_key=fictional-secret\n{}",
        "a".repeat(395),
        "界".repeat(1500)
    );
    let excerpt = matching_excerpt(&activity, true, Some(&raw)).unwrap();
    assert_eq!(excerpt.chars().count(), EXCERPT_CHARS);
    assert!(excerpt.starts_with("[sensitive line omitted]"));
    assert!(excerpt.contains("\n…\n"), "the omitted middle is marked");
    assert_eq!(bounded("short", 1200), "short");
    assert_eq!(bounded(&"x".repeat(2000), 1200).chars().count(), 1200);
    assert!(!excerpt.contains("api_key"));
    assert!(!excerpt.contains("fictional-secret"));
    assert!(matching_excerpt(&activity, false, Some(&raw)).is_none());
    assert!(matching_excerpt(&activity, true, None).is_none());
    assert!(matching_excerpt(&activity, true, Some(" \n ")).is_none());
    let sensitive = ActivitySnapshot {
        process_name: "Bitwarden".into(),
        ..activity
    };
    assert!(matching_excerpt(&sensitive, true, Some("A private vault")).is_none());
}

#[tokio::test]
async fn goal_changes_during_sampling_do_not_force_another_accessibility_read() {
    let (state, activity) = fixture();
    let first = read_at(&state, &activity, Instant::now(), |_| async {
        let mut inner = state.inner.lock().unwrap();
        inner.privacy_revision += 1;
        inner.companion.invalidate();
        Ok("The same foreground document".into())
    })
    .await
    .unwrap();
    assert!(first.is_some());
    let second = read_at(&state, &activity, Instant::now(), |_| async {
        panic!("Goal edits do not invalidate screen evidence")
    })
    .await
    .unwrap();
    assert_eq!(first, second);
}

#[tokio::test]
async fn quiet_mode_keeps_observation_but_pause_private_and_away_stop_it() {
    let (state, activity) = fixture();
    state.inner.lock().unwrap().status.dnd = true;
    assert!(allowed(&state.inner.lock().unwrap(), &activity));
    state
        .inner
        .lock()
        .unwrap()
        .companion
        .view
        .preferences
        .paused = true;
    assert!(!allowed(&state.inner.lock().unwrap(), &activity));
    state
        .inner
        .lock()
        .unwrap()
        .companion
        .view
        .preferences
        .paused = false;
    let away = ActivitySnapshot {
        idle_seconds: 99999,
        ..activity.clone()
    };
    state.inner.lock().unwrap().buddy.foreground = Some(away.clone());
    assert!(!allowed(&state.inner.lock().unwrap(), &away));
    let private = ActivitySnapshot {
        window_title: "Private browsing".into(),
        ..activity
    };
    state.inner.lock().unwrap().buddy.foreground = Some(private.clone());
    assert!(!allowed(&state.inner.lock().unwrap(), &private));
}

#[test]
fn locked_or_secure_session_rejects_even_a_matching_cached_foreground() {
    let (state, activity) = fixture();
    let inner = state.inner.lock().unwrap();
    assert!(allowed_in_session(&inner, &activity, true));
    assert!(!allowed_in_session(&inner, &activity, false));
}

#[test]
fn passive_media_keeps_text_available_but_never_bypasses_lock_exclusion_or_pause() {
    let (state, mut activity) = fixture();
    activity.idle_seconds = 1800;
    activity.media_playing = true;
    let mut inner = state.inner.lock().unwrap();
    inner.buddy.foreground = Some(activity.clone());
    assert!(allowed_in_session(&inner, &activity, true));
    assert!(!allowed_in_session(&inner, &activity, false));
    inner.companion.view.preferences.paused = true;
    assert!(!allowed_in_session(&inner, &activity, true));
    inner.companion.view.preferences.paused = false;
    activity.window_title = "Passwords".into();
    inner.buddy.foreground = Some(activity.clone());
    assert!(!allowed_in_session(&inner, &activity, true));
}

#[tokio::test]
async fn windows_without_accessible_text_are_read_on_device_only_with_screenshot_consent() {
    let (state, activity) = fixture();
    let recognized = AtomicUsize::new(0);
    let recognize = |_: ActivitySnapshot| async {
        recognized.fetch_add(1, Ordering::SeqCst);
        Ok("Maria - Payoneer Recruiting\nAPI_KEY=fictional-private-canary\nTuesday 10-12 works for me".to_string())
    };
    // A messenger without an accessibility tree: only the window chrome is readable.
    state
        .inner
        .lock()
        .unwrap()
        .companion
        .view
        .preferences
        .use_screenshots = true;
    let text = read_with(
        &state,
        &activity,
        Instant::now(),
        |_| async { Ok("Telegram".into()) },
        recognize,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(recognized.load(Ordering::SeqCst), 1);
    assert!(text.starts_with("Telegram\n"));
    assert!(text.contains("Tuesday 10-12 works for me"));
    assert!(
        !text.contains("fictional-private-canary"),
        "recognized text is redacted"
    );
    // Rich interface text is enough on its own.
    let rich = "Release notes ".repeat(10);
    state.inner.lock().unwrap().companion.screen_context = None;
    let text = read_with(
        &state,
        &activity,
        Instant::now(),
        |_| async { Ok(rich.clone()) },
        recognize,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(recognized.load(Ordering::SeqCst), 1);
    assert_eq!(text, rich);
    // Without the screenshot consent the window's pixels are never read.
    {
        let mut inner = state.inner.lock().unwrap();
        inner.companion.view.preferences.use_screenshots = false;
        inner.companion.screen_context = None;
    }
    let text = read_with(
        &state,
        &activity,
        Instant::now(),
        |_| async { Ok(String::new()) },
        recognize,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(recognized.load(Ordering::SeqCst), 1);
    assert!(text.is_empty());
}
