//! One redacted Accessibility sample shared by completion and matching, with an
//! encrypted review copy scoped to its original measured activity segment.
use super::{redact_sample, sensitive_window};
use crate::{
    commands::{AppState, Inner},
    models::ActivitySnapshot,
};
use std::{future::Future, time::Instant};

pub(crate) struct CachedText {
    activity: ActivitySnapshot,
    consent: (u64, u64),
    at: Instant,
    text: Result<String, String>,
}
pub(crate) fn capability(inner: &Inner) -> &'static str {
    match inner.companion.screen_context.as_ref().map(|c| &c.text) {
        Some(Ok(text)) if !text.trim().is_empty() => "available",
        Some(Ok(_)) => "no_accessible_text",
        Some(Err(_)) => "unavailable",
        None => "not_sampled",
    }
}
fn same_window(a: &ActivitySnapshot, b: &ActivitySnapshot) -> bool {
    a.window_id.is_some()
        && a.window_id == b.window_id
        && a.process_name == b.process_name
        && a.window_title == b.window_title
        && a.browser == b.browser
        && a.document_key == b.document_key
}
/// Excerpt budget for one cloud disclosure of visible text. Content regions are read
/// first (see the native walk), so the start holds the document's opening and the end
/// holds the newest lines of a conversation or log; both ends are kept.
pub(crate) const EXCERPT_CHARS: usize = 1200;
/// Keep both ends of a sample within the budget, marking the omitted middle.
pub(crate) fn bounded(text: &str, budget: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= budget {
        return text.into();
    }
    let keep = budget.saturating_sub(3);
    let head = keep / 2;
    let tail = keep - head;
    chars[..head]
        .iter()
        .chain(['\n', '…', '\n'].iter())
        .chain(chars[chars.len() - tail..].iter())
        .collect()
}
pub(crate) fn allowed(inner: &Inner, activity: &ActivitySnapshot) -> bool {
    allowed_in_session(inner, activity, crate::collector::session_active())
}
/// A fingerprint of the text last read from this exact window, when one is cached.
/// Callers decide whether evidence is new without touching the text itself.
pub(crate) fn cached_signature(inner: &Inner, activity: &ActivitySnapshot) -> Option<String> {
    inner
        .companion
        .screen_context
        .as_ref()
        .filter(|c| same_window(&c.activity, activity))
        .and_then(|c| c.text.as_ref().ok())
        .map(|text| crate::companion::engine::fingerprint(text))
}
/// Text last read from this exact window under the current consent, for local phrase
/// checks only. Nothing is read here; a window without a recent sample has no text.
pub(crate) fn cached_text<'a>(inner: &'a Inner, activity: &ActivitySnapshot) -> Option<&'a str> {
    inner
        .companion
        .screen_context
        .as_ref()
        .filter(|c| {
            c.consent
                == (
                    inner.last_consent_change,
                    inner.companion.screen_context_epoch,
                )
                && same_window(&c.activity, activity)
        })
        .and_then(|c| c.text.as_ref().ok())
        .map(String::as_str)
}
#[cfg(test)]
pub(crate) fn cache_for_test(inner: &mut Inner, activity: &ActivitySnapshot, text: &str) {
    inner.companion.screen_context = Some(CachedText {
        activity: activity.clone(),
        consent: (
            inner.last_consent_change,
            inner.companion.screen_context_epoch,
        ),
        at: Instant::now(),
        text: Ok(text.into()),
    });
}
fn allowed_in_session(inner: &Inner, activity: &ActivitySnapshot, session_active: bool) -> bool {
    session_active
        && inner.status.tracking
        && !inner.status.demo
        && !inner.status.mock_ai
        && inner.companion.view.preferences.screen_task_detection
        && inner.storage.ai_preferences().is_ok_and(|p| p.enabled)
        && !inner.companion.view.preferences.paused
        && (activity.engaged(inner.tracking_settings.idle_seconds)
            || inner.companion.semantic.terminal_grace(activity))
        && !sensitive_window(activity)
        && !crate::attention::excluded(&inner.buddy.view.preferences, &activity.process_name)
        && inner
            .buddy
            .foreground
            .as_ref()
            .is_some_and(|a| same_window(a, activity))
}
/// Redact complete lines before truncating, including a secret that crosses the cutoff.
pub(crate) fn matching_excerpt(
    activity: &ActivitySnapshot,
    enabled: bool,
    text: Option<&str>,
) -> Option<String> {
    if !enabled || sensitive_window(activity) {
        return None;
    }
    text.map(redact_sample)
        .filter(|s| !s.trim().is_empty())
        .map(|s| bounded(&s, EXCERPT_CHARS))
}
/// Interface text this short (letters and digits) is supplemented by on-device recognition.
const THIN_TEXT: usize = 80;
fn thin(text: &Result<String, String>) -> bool {
    text.as_ref().map_or(true, |t| {
        t.chars().filter(|c| c.is_alphanumeric()).count() < THIN_TEXT
    })
}
/// Recognition needs the screenshot consent; the image stays on the device and is not kept.
fn recognition_allowed(inner: &Inner) -> bool {
    inner.companion.view.preferences.use_screenshots
}
pub(crate) async fn read(
    state: &AppState,
    activity: &ActivitySnapshot,
) -> Result<Option<String>, String> {
    read_with(
        state,
        activity,
        Instant::now(),
        super::sample,
        |expected| async move {
            tauri::async_runtime::spawn_blocking(move || crate::vision::recognize_window(&expected))
                .await
                .map_err(|_| "Text recognition unavailable".to_string())?
        },
    )
    .await
}
#[cfg(test)]
async fn read_at<F: Future<Output = Result<String, String>>>(
    state: &AppState,
    activity: &ActivitySnapshot,
    now: Instant,
    sample: impl FnOnce(usize) -> F,
) -> Result<Option<String>, String> {
    read_with(state, activity, now, sample, |_| async {
        Err("Text recognition unavailable".to_string())
    })
    .await
}
async fn read_with<
    F: Future<Output = Result<String, String>>,
    R: Future<Output = Result<String, String>>,
>(
    state: &AppState,
    activity: &ActivitySnapshot,
    now: Instant,
    sample: impl FnOnce(usize) -> F,
    recognize: impl FnOnce(ActivitySnapshot) -> R,
) -> Result<Option<String>, String> {
    // Neither caller holds Inner across an await. Concurrent consumers share one read,
    // including unavailable samples; the cache is never serialized or persisted.
    let waiting = Instant::now();
    let _read = state.screen_context_read.lock().await;
    let now = now + waiting.elapsed();
    let consent = {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "Application state unavailable")?;
        if !allowed(&inner, activity) {
            inner.companion.screen_context = None;
            return Ok(None);
        }
        if let Some(cached) = &inner.companion.screen_context {
            if cached.consent
                == (
                    inner.last_consent_change,
                    inner.companion.screen_context_epoch,
                )
                && same_window(&cached.activity, activity)
                && now.saturating_duration_since(cached.at) < crate::observation::SAMPLE_INTERVAL
            {
                return cached.text.clone().map(Some);
            }
        }
        (
            inner.last_consent_change,
            inner.companion.screen_context_epoch,
        )
    };
    let mut text = sample(activity.window_id.ok_or("No foreground window")? as usize)
        .await
        .map(|raw| redact_sample(&raw));
    // Apps without an accessible text tree (many messengers, canvases, remote screens) are
    // read from the window's own pixels on this device instead.
    let recognize_now = thin(&text) && {
        let inner = state
            .inner
            .lock()
            .map_err(|_| "Application state unavailable".to_string())?;
        recognition_allowed(&inner)
    };
    if recognize_now {
        if let Ok(seen) = recognize(activity.clone()).await {
            let seen = redact_sample(&seen);
            if !seen.trim().is_empty() {
                text = Ok(match text {
                    Ok(own) if !own.trim().is_empty() => format!("{own}\n{seen}"),
                    _ => seen,
                });
            }
        }
    }
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if !allowed(&inner, activity)
        || (
            inner.last_consent_change,
            inner.companion.screen_context_epoch,
        ) != consent
    {
        inner.companion.screen_context = None;
        return Ok(None);
    }
    if let Ok(text) = &text {
        crate::goal_matching::observe_text(&mut inner, activity, text);
        if let Err(error) = inner.storage.retain_review_evidence(activity, "text", text) {
            inner.last_error = Some(error);
        }
    }
    inner.companion.screen_context = Some(CachedText {
        activity: activity.clone(),
        consent,
        at: now,
        text: text.clone(),
    });
    text.map(Some)
}

#[cfg(test)]
mod tests;
