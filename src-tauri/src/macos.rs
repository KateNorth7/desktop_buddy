//! Public macOS APIs behind a small, owned-string native boundary.
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::OnceLock;
use tauri::{Manager, WebviewWindow};

#[allow(
    clippy::duplicated_attributes,
    reason = "each #[link] names a different library; the shared kind is not a duplicate"
)]
#[link(name = "buddy_macos", kind = "static")]
#[link(name = "AppKit", kind = "framework")]
#[link(name = "ApplicationServices", kind = "framework")]
#[link(name = "Security", kind = "framework")]
#[link(name = "Carbon", kind = "framework")]
#[link(name = "Speech", kind = "framework")]
#[link(name = "CoreAudio", kind = "framework")]
#[link(name = "ScreenCaptureKit", kind = "framework")]
extern "C" {
    fn buddy_free(value: *mut c_char);
    fn buddy_accessibility(prompt: bool) -> bool;
    fn buddy_session_active() -> bool;
    fn buddy_idle() -> f64;
    fn buddy_foreground(metadata: bool) -> *mut c_char;
    fn buddy_fullscreen() -> bool;
    fn buddy_pin_window(window: *mut c_void);
    fn buddy_show_without_focus(window: *mut c_void);
    fn buddy_context(window: u64, selected: bool) -> *mut c_char;
    fn buddy_screen_permission(request: bool) -> bool;
    fn buddy_capture_window(window: u64) -> *mut c_char;
    #[cfg(not(test))]
    fn buddy_review_key(secret: *const c_char) -> *mut c_char;
    fn buddy_apps() -> *mut c_char;
    fn buddy_keychain(account: *const c_char, secret: *const c_char, operation: i32)
        -> *mut c_char;
    fn buddy_shortcuts(callback: extern "C" fn(u32)) -> u32;
    // Only installed release builds register a login item; see `autostart::sync`.
    #[cfg(not(debug_assertions))]
    fn buddy_autostart(executable: *const c_char, enabled: bool) -> *mut c_char;
}
fn decode<T: DeserializeOwned>(pointer: *mut c_char) -> Result<T, String> {
    if pointer.is_null() {
        return Err("macOS integration is unavailable".into());
    }
    // Copy before freeing the native allocation. This data is never logged.
    let bytes = unsafe { CStr::from_ptr(pointer).to_bytes().to_vec() };
    unsafe { buddy_free(pointer) };
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid macOS integration response")?;
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
        return Err(error.replace("Desktop Buddy", "sumi"));
    }
    serde_json::from_value(value).map_err(|_| "Invalid macOS integration response".into())
}
fn string(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| "Invalid macOS integration input".into())
}
/// Independent local evidence key. Provider credentials continue to use the
/// immutable signed helper and its unchanged allowlist.
#[cfg(not(test))]
pub(crate) fn review_key(secret: Option<&str>) -> Result<Option<String>, String> {
    let secret = secret.map(string).transpose()?;
    decode(unsafe { buddy_review_key(secret.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())) })
}
#[derive(Deserialize)]
pub struct Foreground {
    pub process: String,
    pub title: String,
    pub window: u64,
    pub idle: f64,
    pub media: bool,
    pub address: Option<String>,
    /// Document identity of the focused window (an AXDocument path, or a web app's
    /// document URL without query or fragment). Hashed by the collector; never stored raw.
    #[serde(default)]
    pub document: Option<String>,
}
/// Registers or removes the macOS login item. Called only from release builds
/// (`autostart::sync`), so debug Clippy and tests never compile a caller.
#[cfg(not(debug_assertions))]
pub fn autostart(enabled: bool) -> Result<(), String> {
    let path = std::env::current_exe().map_err(|_| "Could not locate the installed application")?;
    let path = string(path.to_str().ok_or("Invalid application path")?)?;
    decode(unsafe { buddy_autostart(path.as_ptr(), enabled) })
}
pub fn foreground(metadata: bool) -> Result<Foreground, String> {
    decode(unsafe { buddy_foreground(metadata) })
}
pub fn fullscreen() -> bool {
    unsafe { buddy_fullscreen() }
}
pub fn idle_seconds() -> Option<u64> {
    let idle = unsafe { buddy_idle() };
    (idle.is_finite() && idle >= 0.0).then_some(idle as u64)
}
pub fn context(window: usize, selected: bool) -> Result<String, String> {
    decode(unsafe { buddy_context(window as u64, selected) })
}
pub fn installed_apps() -> Result<Vec<crate::installed_apps::InstalledApp>, String> {
    decode(unsafe { buddy_apps() })
}
pub fn read_key(name: &str) -> Result<Option<String>, String> {
    let name = string(name)?;
    decode(unsafe { buddy_keychain(name.as_ptr(), std::ptr::null(), 0) })
}
pub fn has_key(name: &str) -> Result<bool, String> {
    let name = string(name)?;
    decode(unsafe { buddy_keychain(name.as_ptr(), std::ptr::null(), 3) })
}
pub fn write_key(name: &str, key: Option<&str>) -> Result<(), String> {
    let name = string(name)?;
    let key = key.map(string).transpose()?;
    decode(unsafe {
        buddy_keychain(
            name.as_ptr(),
            key.as_ref().map_or(std::ptr::null(), |k| k.as_ptr()),
            if key.is_some() { 1 } else { 2 },
        )
    })
}
pub fn request_accessibility_permission(window: WebviewWindow) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("Open Settings in the workspace to manage permissions".into());
    }
    let allowed = unsafe { buddy_accessibility(true) };
    if !allowed {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .status()
            .map_err(|_| "Could not open Accessibility settings. Open System Settings → Privacy & Security → Accessibility.")?;
        if !status.success() {
            return Err("Could not open Accessibility settings. Open System Settings → Privacy & Security → Accessibility.".into());
        }
    }
    Ok(allowed)
}
/// Legacy ad-hoc builds identify an exact binary hash, so their grants may become stale after an
/// update or signing-identity transition. This explicit repair removes only sumi's old grant.
/// Normal updates signed with the same stable identity must not invoke this reset.
pub fn reset_accessibility_permission(
    app: &tauri::AppHandle,
    window: WebviewWindow,
) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("Open Settings in the workspace to manage permissions".into());
    }
    let identifier = app.config().identifier.clone();
    let status = std::process::Command::new("/usr/bin/tccutil")
        .args(["reset", "Accessibility", &identifier])
        .status()
        .map_err(|_| "Could not reset the Accessibility permission")?;
    if !status.success() {
        return Err("macOS did not reset the Accessibility permission".into());
    }
    Ok(unsafe { buddy_accessibility(true) })
}
pub fn pin_window(window: &WebviewWindow) {
    if let Ok(handle) = window.ns_window() {
        unsafe { buddy_pin_window(handle) };
    }
}
pub fn show_without_focus(window: &WebviewWindow) -> Result<(), String> {
    let handle = window
        .ns_window()
        .map_err(|_| "Could not read sumi native window")?;
    unsafe { buddy_show_without_focus(handle) };
    Ok(())
}
pub fn get_accessibility_permission() -> bool {
    unsafe { buddy_accessibility(false) }
}
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
extern "C" fn selected_shortcut(kind: u32) {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    // Never hold application state on the native event thread.
    tauri::async_runtime::spawn(async move {
        let allowed = app
            .state::<crate::commands::AppState>()
            .inner
            .lock()
            .is_ok_and(|inner| {
                inner
                    .storage
                    .user_settings()
                    .is_ok_and(|s| s.onboarding.completed)
            });
        if !allowed || fullscreen() {
            return;
        }
        let result = tauri::async_runtime::spawn_blocking(|| {
            let active = foreground(false)?;
            context(active.window as usize, true)
        })
        .await;
        let (text, mut notice) = match result {
            Ok(Ok(text)) if !text.trim().is_empty() => (text, None),
            _ => (String::new(), Some("Selected text is unavailable. Allow Accessibility access or paste it into sumi.".to_string())),
        };
        if kind == 2 && !text.is_empty() {
            match crate::core_capture::receive(&app, &text) {
                Ok(()) => return,
                Err(error) => notice = Some(error),
            }
        }
        if let Ok(mut inner) = app.state::<crate::commands::AppState>().inner.lock() {
            inner.companion.view.notice = notice;
            inner.companion.view.seed = text;
            inner.companion.view.intent = "selection".into();
            inner.companion.view.chat_open = true;
            let _ = crate::buddy::sync(&app, &mut inner);
        }
        let _ = crate::buddy::windows::focus_chat(&app);
    });
}
pub fn install_shortcuts(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
    let available = unsafe { buddy_shortcuts(selected_shortcut) };
    if let Ok(mut inner) = app.state::<crate::commands::AppState>().inner.lock() {
        inner.companion.view.shortcut_available = available & 1 != 0;
    }
}

/// A browser window between pages ("Untitled", the browser's own name, or a titled page
/// whose address has not arrived yet) is skipped for this long before it counts as a
/// context of its own, so one page visit is not split into three.
const TRANSITIONAL_GRACE: std::time::Duration = std::time::Duration::from_secs(6);
#[derive(Default)]
pub struct MacCollector {
    tracker: crate::collector::DurationTracker,
    previous_window: u64,
    settings: crate::tracking::TrackingSettings,
    excluded: Vec<String>,
    transitional_since: Option<std::time::Instant>,
}
fn transitional(process: &str, title: &str, domain: Option<&str>) -> bool {
    let title = title.trim();
    crate::browser::browser_name(process).is_some()
        && domain.is_none_or(str::is_empty)
        && (title.is_empty()
            || title.eq_ignore_ascii_case(process)
            || title.eq_ignore_ascii_case("untitled")
            || title.eq_ignore_ascii_case("new tab")
            || title.to_ascii_lowercase().starts_with("loading"))
}
impl crate::collector::ActivityCollector for MacCollector {
    fn configure(&mut self, settings: &crate::tracking::TrackingSettings, excluded: &[String]) {
        self.settings = settings.clone();
        self.excluded = excluded.to_vec();
    }
    fn collect(&mut self) -> Result<crate::models::ActivitySnapshot, String> {
        if !session_active() {
            self.tracker = Default::default();
            return Err("Desktop session is inactive or locked".into());
        }
        let active = foreground(false)?;
        if self.previous_window != active.window {
            self.tracker = Default::default();
            self.previous_window = active.window;
        }
        let excluded = self
            .excluded
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&active.process));
        // Exclusions are checked before reading any browser address metadata. Only a
        // browser gets the second, address-bearing read; every other application is
        // sampled once per tick.
        let active = if self.settings.browser_metadata
            && !excluded
            && crate::browser::browser_name(&active.process).is_some()
        {
            let metadata = foreground(true)?;
            if active.window != metadata.window || active.title != metadata.title {
                self.tracker = Default::default();
                return Err(crate::collector::CONTEXT_CHANGED.into());
            }
            metadata
        } else {
            active
        };
        let domain = active
            .address
            .as_deref()
            .and_then(crate::browser::sanitize_domain);
        if transitional(&active.process, &active.title, domain.as_deref()) {
            let since = *self
                .transitional_since
                .get_or_insert_with(std::time::Instant::now);
            if since.elapsed() < TRANSITIONAL_GRACE {
                return Err(crate::collector::CONTEXT_CHANGED.into());
            }
        } else {
            self.transitional_since = None;
        }
        let browser = crate::browser::context(&active.process, &active.title, domain);
        let idle = if active.idle.is_finite() && active.idle >= 0.0 {
            active.idle as u64
        } else {
            return Err("Could not read idle time".into());
        };
        let context = format!(
            "{}|{}",
            active.title,
            browser
                .as_ref()
                .and_then(|b| b.domain.as_deref())
                .unwrap_or("")
        );
        // Browsers are identified by domain and page title as before. Other applications
        // whose windows share one title (chat apps, editors, web apps) are told apart by a
        // hash of their document identity; the raw value is dropped here.
        let document_key = active
            .document
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty() && browser.is_none())
            .map(crate::companion::engine::fingerprint);
        let context = format!("{context}|{}", document_key.as_deref().unwrap_or_default());
        let duration =
            self.tracker
                .update(&active.process, &context, idle, self.settings.idle_seconds);
        Ok(crate::models::ActivitySnapshot {
            timestamp: chrono::Utc::now().to_rfc3339(),
            process_name: active.process,
            window_title: browser
                .as_ref()
                .map_or(active.title, |b| b.page_title.clone()),
            idle_seconds: idle,
            active_seconds: duration,
            window_id: Some(active.window),
            browser,
            media_playing: active.media,
            document_key,
        })
    }
}

pub(crate) fn session_active() -> bool {
    unsafe { buddy_session_active() }
}
pub(crate) fn screen_permission(request: bool) -> bool {
    unsafe { buddy_screen_permission(request) }
}
use crate::vision::{CaptureFailure, CaptureFailureKind};
/// Native capture failures arrive as `{error, code, detail}`; the code is one of the stable
/// categories written by `buddy_capture_window`, the detail names its stage. Neither is
/// screen content, and unknown codes degrade to the generic category.
fn decode_capture(pointer: *mut c_char) -> Result<String, CaptureFailure> {
    if pointer.is_null() {
        return Err(CaptureFailure::new(
            CaptureFailureKind::Unavailable,
            "native_capture_returned_nothing",
        ));
    }
    let bytes = unsafe { CStr::from_ptr(pointer).to_bytes().to_vec() };
    unsafe { buddy_free(pointer) };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        CaptureFailure::new(
            CaptureFailureKind::Unavailable,
            "native_capture_response_invalid",
        )
    })?;
    if let Some(code) = value.get("code").and_then(|v| v.as_str()) {
        let detail = value.get("detail").and_then(|v| v.as_str()).unwrap_or("");
        return Err(CaptureFailure::new(
            capture_failure_kind(code),
            capture_detail(detail),
        ));
    }
    if value.get("error").is_some() {
        return Err(CaptureFailure::new(
            CaptureFailureKind::Unavailable,
            "native_capture_failed",
        ));
    }
    value.as_str().map(str::to_owned).ok_or(CaptureFailure::new(
        CaptureFailureKind::Unavailable,
        "native_capture_response_invalid",
    ))
}
pub(crate) fn capture_failure_kind(code: &str) -> CaptureFailureKind {
    use CaptureFailureKind::*;
    match code {
        "session_inactive" => SessionInactive,
        "permission_unavailable" => PermissionUnavailable,
        "foreground_changed" => ForegroundChanged,
        "window_minimized" => WindowMinimized,
        "window_unavailable" => WindowUnavailable,
        "protected_content" => ProtectedContent,
        "window_occluded" => Occluded,
        "blank_frame" => BlankFrame,
        "capture_timeout" => Timeout,
        "capture_unsupported" => Unsupported,
        "api_error" => ApiError,
        "encoding_failed" => Encoding,
        "image_too_large" => ImageTooLarge,
        _ => Unavailable,
    }
}
/// Stage details are a closed vocabulary written by the native adapter.
pub(crate) fn capture_detail(detail: &str) -> &'static str {
    const KNOWN: [&str; 19] = [
        "session_inactive_or_locked",
        "screen_recording_not_granted",
        "focused_window_identity_unresolved",
        "frontmost_application_differs",
        "target_window_closed_or_stale",
        "target_window_minimized",
        "focused_window_unavailable",
        "focused_window_differs",
        "focused_window_minimized",
        "focused_window_bounds_unavailable",
        "focused_window_not_on_screen",
        "shareable_content_unavailable",
        "capture_window_not_resolved",
        "screenshot_manager_failed",
        "session_locked_during_capture",
        "frontmost_application_changed_during_capture",
        "focused_window_changed_during_capture",
        "jpeg_encoding_failed",
        "jpeg_over_budget",
    ];
    KNOWN
        .iter()
        .find(|known| **known == detail)
        .copied()
        .unwrap_or("native_stage_unknown")
}
pub(crate) fn capture_window(window: u64) -> Result<String, CaptureFailure> {
    if window & 0xFFFF_FFFF == 0 {
        return Err(CaptureFailure::new(
            CaptureFailureKind::WindowUnavailable,
            "focused_window_identity_unresolved",
        ));
    }
    let current = |detail| match foreground(false) {
        Ok(active) if active.window == window => Ok(()),
        Ok(_) => Err(CaptureFailure::new(
            CaptureFailureKind::ForegroundChanged,
            detail,
        )),
        Err(_) => Err(CaptureFailure::new(
            CaptureFailureKind::WindowUnavailable,
            "foreground_collection_unavailable",
        )),
    };
    current("foreground_changed_before_capture")?;
    let image = decode_capture(unsafe { buddy_capture_window(window) })?;
    current("foreground_changed_after_capture")?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_inventory_does_not_return_paths() {
        for app in installed_apps().unwrap() {
            assert!(!app.process_name.contains('/'));
            assert!(!app.name.contains("/Applications/"));
        }
    }
    #[test]
    fn page_loads_are_transitional_only_for_browsers_without_an_address() {
        assert!(transitional("Safari", "Safari", None));
        assert!(transitional("Safari", "Untitled", Some("")));
        assert!(transitional("Google Chrome", "Loading…", None));
        assert!(!transitional("Safari", "Register – L’Oréal Groupe", None));
        assert!(!transitional("Safari", "Untitled", Some("example.com")));
        assert!(!transitional("Claude", "Claude", None));
        assert!(!transitional("Telegram", "", None));
    }
    #[test]
    fn idle_is_a_valid_duration() {
        assert!(idle_seconds().is_some());
    }
    #[test]
    fn native_capture_failures_map_to_sanitized_categories() {
        let failure = |body: &str| {
            let raw = std::ffi::CString::new(body).unwrap().into_raw();
            decode_capture(raw).unwrap_err()
        };
        let f = failure(
            r#"{"error":"Window capture unavailable","code":"permission_unavailable","detail":"screen_recording_not_granted"}"#,
        );
        assert_eq!(f.kind, CaptureFailureKind::PermissionUnavailable);
        assert_eq!(f.detail, "screen_recording_not_granted");
        assert_eq!(f.status(), "permission_unavailable");
        let f = failure(
            r#"{"error":"Window capture unavailable","code":"window_minimized","detail":"focused_window_minimized"}"#,
        );
        assert_eq!(f.kind, CaptureFailureKind::WindowMinimized);
        // Unknown codes and details never pass through verbatim.
        let f = failure(r#"{"error":"x","code":"Secret title","detail":"Private detail"}"#);
        assert_eq!(f.kind, CaptureFailureKind::Unavailable);
        assert_eq!(f.detail, "native_stage_unknown");
        let f = failure(r#"{"error":"Screen context changed"}"#);
        assert_eq!(f.detail, "native_capture_failed");
        assert_eq!(
            f.to_string(),
            "Window capture unavailable (native_capture_failed)"
        );
        let raw = std::ffi::CString::new(r#""aW1hZ2U=""#).unwrap().into_raw();
        assert_eq!(decode_capture(raw).unwrap(), "aW1hZ2U=");
    }
    #[test]
    fn stale_or_unresolved_window_identities_never_reach_native_capture() {
        // Identity 1 belongs to pid 0 and cannot be the live frontmost window.
        let failure = capture_window(1).unwrap_err();
        assert!(matches!(
            failure.kind,
            CaptureFailureKind::ForegroundChanged | CaptureFailureKind::WindowUnavailable
        ));
        assert!(!failure.detail.is_empty());
        // A zero window hash means the collector never resolved a focused window.
        let failure = capture_window(u64::from(std::process::id()) << 32).unwrap_err();
        assert_eq!(failure.kind, CaptureFailureKind::WindowUnavailable);
        assert_eq!(failure.detail, "focused_window_identity_unresolved");
    }
    #[test]
    #[ignore = "Uses the reviewed local signing identity for an isolated cross-build Keychain test"]
    fn keychain_round_trip() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/test-credential-helper.sh");
        assert!(std::process::Command::new("bash")
            .arg(script)
            .status()
            .unwrap()
            .success());
    }
}
