//! Shared budget and capability diagnostics for foreground-only observation.
use crate::commands::Inner;
use std::time::{Duration, Instant};
/// Minimum interval for foreground text refresh, image capture and goal observation.
pub(crate) const SAMPLE_INTERVAL: Duration = Duration::from_secs(20);
/// The one cost ceiling for background model calls. Everything else that used to pace
/// requests was a cooldown; repeats are now prevented by evidence fingerprints instead.
pub(crate) const HOURLY_LIMIT: usize = 120;
/// Shares of the hour, as headroom each consumer must leave for the ones above it. Goal
/// attribution, outcome verification and the daily review may use the whole budget: they
/// are how measured time reaches a goal. Routine sampling and image descriptions stop
/// earlier, so a busy hour of changing text can no longer starve attribution.
const SECONDARY_HEADROOM: usize = HOURLY_LIMIT / 10;
const VISION_HEADROOM: usize = HOURLY_LIMIT / 3;
const OBSERVATION_HEADROOM: usize = HOURLY_LIMIT / 2;
/// Goal attribution, verification and the daily review.
pub(crate) fn reserve(inner: &mut Inner) -> bool {
    reserve_with_headroom(inner, 0)
}
/// The unlisted-work classifier: attribution too, but it offers rather than assigns.
pub(crate) fn reserve_secondary(inner: &mut Inner) -> bool {
    reserve_with_headroom(inner, SECONDARY_HEADROOM)
}
/// A screenshot description enriching a sample whose text is empty.
pub(crate) fn reserve_vision(inner: &mut Inner) -> bool {
    reserve_with_headroom(inner, VISION_HEADROOM)
}
/// The routine unified observation pass: at most half the hour.
pub(crate) fn reserve_primary(inner: &mut Inner) -> bool {
    reserve_with_headroom(inner, OBSERVATION_HEADROOM)
}
fn reserve_with_headroom(inner: &mut Inner, headroom: usize) -> bool {
    let now = Instant::now();
    inner
        .companion
        .observation_calls
        .retain(|at| now.saturating_duration_since(*at) < Duration::from_secs(3600));
    if inner.companion.observation_calls.len() >= HOURLY_LIMIT.saturating_sub(headroom) {
        crate::engine_observability::record(
            inner,
            "suppress",
            "observation_budget",
            "hourly_limit",
            None,
            None,
        );
        return false;
    }
    inner.companion.observation_calls.push_back(now);
    true
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn attribution_keeps_budget_that_routine_sampling_and_vision_cannot_spend() {
        let state = crate::commands::AppState::new(
            crate::storage::Storage::open(std::path::Path::new(":memory:")).unwrap(),
        )
        .unwrap();
        let mut inner = state.inner.lock().unwrap();
        let spend = |inner: &mut Inner, f: fn(&mut Inner) -> bool| {
            let mut n = 0;
            while f(inner) {
                n += 1;
                assert!(n <= HOURLY_LIMIT);
            }
            n
        };
        assert_eq!(spend(&mut inner, reserve_primary), HOURLY_LIMIT / 2);
        assert_eq!(
            spend(&mut inner, reserve_vision),
            HOURLY_LIMIT / 2 - HOURLY_LIMIT / 3
        );
        assert_eq!(
            spend(&mut inner, reserve_secondary),
            HOURLY_LIMIT / 3 - HOURLY_LIMIT / 10
        );
        assert_eq!(spend(&mut inner, reserve), HOURLY_LIMIT / 10);
        assert!(!reserve(&mut inner));
        // An hour later the whole budget is available again.
        let hour_ago = Instant::now() - Duration::from_secs(3601);
        for at in inner.companion.observation_calls.iter_mut() {
            *at = hour_ago;
        }
        assert!(reserve_primary(&mut inner));
    }
}
pub(crate) fn personal_context(
    inner: &Inner,
    request: &str,
    goal: Option<i64>,
) -> serde_json::Value {
    let share = inner
        .storage
        .ai_preferences()
        .is_ok_and(|p| p.enabled && p.share_goal_context);
    inner
        .storage
        .retrieve_contextual_memory(
            &crate::memory::retriever::RetrievalInput {
                user_request: request.chars().take(1000).collect(),
                active_goal_id: goal,
                ..Default::default()
            },
            &crate::memory::retriever::MemoryBudget {
                maximum_memories: 4,
                maximum_context_size: 1800,
                ..Default::default()
            },
            share,
        )
        .map(|m| m.context)
        .unwrap_or(serde_json::Value::Null)
}

/// Operational facts, not screen contents or private history, ground capability explanations in chat.
pub(crate) fn capabilities(inner: &Inner) -> serde_json::Value {
    let p = &inner.companion.view.preferences;
    let ai = inner.storage.ai_preferences().unwrap_or_default();
    serde_json::json!({
        "tracking_enabled": inner.status.tracking,
        "tracking_has_error": inner.status.tracking_error.is_some(),
        "ai_enabled": ai.enabled,
        "provider_key_configured": inner.status.nebius_configured,
        "observation_enabled": crate::semantic_observation::owns_observation(inner),
        "goal_sharing_enabled": ai.share_goal_context,
        "interface_observation_enabled": p.screen_task_detection,
        "interface_text_status": crate::companion::screen_context::capability(inner),
        "screenshots_enabled": p.use_screenshots,
        "screenshot_status": if inner.vision.status.is_empty() {"not_sampled"} else {inner.vision.status},
        "screenshot_detail": inner.vision.detail,
        "automatic_completion_enabled": p.automatic_completion,
        "observation_paused": p.paused,
        "quiet_mode": crate::companion::quiet_mode(inner),
        "last_check_failed": inner.companion.progress_error.is_some(),
        "latest_observation": if ai.share_goal_context {inner.companion.semantic.diagnostics()} else {serde_json::Value::Null},
        "rule": "Settings express consent, not proof of a successful capture. Screen observation is supported for permitted foreground apps; hidden/offline actions are not visible. Automatic completion is conditional on its opt-in and sufficient independently checked outcome evidence. Do not say all completion needs manual confirmation or that screen access is absent when these facts do not establish that. Quiet mode silences presentation; it is separate from tracking consent. New goals and steps require user acceptance. Time spent or a deadline alone never establishes completion."
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hourly_budget_caps_disclosures_but_expired_entries_do_not_block_work() {
        let state = crate::commands::AppState::new(
            crate::storage::Storage::open(std::path::Path::new(":memory:")).unwrap(),
        )
        .unwrap();
        let mut i = state.inner.lock().unwrap();
        for _ in 0..HOURLY_LIMIT {
            assert!(reserve(&mut i));
        }
        assert!(!reserve(&mut i));
        assert_eq!(i.companion.observation_calls.len(), HOURLY_LIMIT);
        i.companion.observation_calls.clear();
        i.companion
            .observation_calls
            .push_back(Instant::now() - Duration::from_secs(3601));
        assert!(reserve(&mut i));
        assert_eq!(i.companion.observation_calls.len(), 1);
    }
    #[test]
    fn primary_sampling_stops_at_half_the_hour_and_hides_goal_diagnostics_without_sharing() {
        let state = crate::commands::AppState::new(
            crate::storage::Storage::open(std::path::Path::new(":memory:")).unwrap(),
        )
        .unwrap();
        let mut i = state.inner.lock().unwrap();
        for _ in 0..HOURLY_LIMIT / 2 {
            assert!(reserve_primary(&mut i));
        }
        assert!(!reserve_primary(&mut i));
        for _ in 0..HOURLY_LIMIT / 2 {
            assert!(reserve(&mut i));
        }
        assert!(!reserve(&mut i));
        i.companion.semantic.status = "completed";
        let mut ai = i.storage.ai_preferences().unwrap();
        ai.enabled = true;
        ai.share_goal_context = true;
        i.storage
            .write_setting("ai_assistance_preferences", &ai)
            .unwrap();
        i.companion.view.preferences.use_screenshots = true;
        i.companion.view.preferences.automatic_completion = true;
        let status = capabilities(&i);
        assert_eq!(status["screenshots_enabled"], true);
        assert_eq!(status["automatic_completion_enabled"], true);
        assert_eq!(status["latest_observation"]["status"], "completed");
        ai.share_goal_context = false;
        i.storage
            .write_setting("ai_assistance_preferences", &ai)
            .unwrap();
        assert!(capabilities(&i)["latest_observation"].is_null());
        assert_eq!(capabilities(&i)["observation_enabled"], false);
    }
}
