use crate::models::ActivitySnapshot;
use chrono::Utc;
pub const CONTEXT_CHANGED: &str = "Foreground context changed during collection";
#[cfg(windows)]
mod browser_windows;
#[cfg(windows)]
mod windows;

pub trait ActivityCollector: Send {
    fn configure(&mut self, _settings: &crate::tracking::TrackingSettings, _excluded: &[String]) {}
    fn collect(&mut self) -> Result<ActivitySnapshot, String>;
}
/// Never disclose a cached sample after switching to a locked/secure desktop.
pub(crate) fn session_active() -> bool {
    if cfg!(test) {
        return true; // Unit fixtures use the explicit session-policy seam instead of host state.
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::session_active()
    }
    #[cfg(windows)]
    {
        windows::session_active()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}
/// Fresh metadata check around pixel capture; never trust a collector-tick cache alone.
pub(crate) fn matches_foreground(expected: &ActivitySnapshot) -> bool {
    create(false).collect().is_ok_and(|live| {
        live.window_id.is_some()
            && live.window_id == expected.window_id
            && live.process_name == expected.process_name
            && crate::browser::minimize_title(&live.window_title)
                == crate::browser::minimize_title(&expected.window_title)
            && !crate::companion::sensitive_window(&live)
    })
}
pub fn foreground_fullscreen() -> bool {
    #[cfg(windows)]
    {
        windows::foreground_fullscreen()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::fullscreen()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}
pub fn foreground_meeting() -> bool {
    #[cfg(windows)]
    {
        windows::foreground_meeting()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::foreground(false).is_ok_and(|active| {
            crate::attention::meeting(&ActivitySnapshot {
                process_name: active.process,
                window_title: active.title,
                ..Default::default()
            })
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}
pub fn foreground_idle_seconds() -> Option<u64> {
    #[cfg(windows)]
    {
        windows::foreground_idle_seconds()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::idle_seconds()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

#[derive(Default)]
pub struct DurationTracker {
    previous: Option<(String, String, std::time::Instant, std::time::Duration)>,
}
impl DurationTracker {
    pub fn update(&mut self, process: &str, title: &str, idle: u64, idle_threshold: u64) -> u64 {
        let now = std::time::Instant::now();
        let duration = self
            .previous
            .as_ref()
            .map(|(p, t, last, accumulated)| {
                let elapsed = now.duration_since(*last);
                if p == process
                    && t == title
                    && idle < idle_threshold
                    && elapsed <= std::time::Duration::from_secs(15)
                {
                    *accumulated + elapsed
                } else {
                    std::time::Duration::ZERO
                }
            })
            .unwrap_or_default();
        self.previous = Some((process.into(), title.into(), now, duration));
        duration.as_secs()
    }
}
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "the Windows collector reads process paths; tests run on every platform"
    )
)]
pub fn process_basename(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or("Unknown")
        .to_string()
}
pub fn create(demo: bool) -> Box<dyn ActivityCollector> {
    if demo {
        return Box::new(DemoCollector { step: 0 });
    }
    #[cfg(windows)]
    {
        Box::new(windows::WindowsCollector::default())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(crate::macos::MacCollector::default())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Box::new(UnsupportedCollector)
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
struct UnsupportedCollector;
#[cfg(not(any(windows, target_os = "macos")))]
impl ActivityCollector for UnsupportedCollector {
    fn collect(&mut self) -> Result<ActivitySnapshot, String> {
        Err("Live activity collection requires Windows or macOS".into())
    }
}
struct DemoCollector {
    step: usize,
}
impl ActivityCollector for DemoCollector {
    fn collect(&mut self) -> Result<ActivitySnapshot, String> {
        let items = [
            ("POWERPNT.EXE", "Hackathon.pptx — PowerPoint", 600),
            ("chrome.exe", "Claude Docs", 300),
            ("chrome.exe", "YouTube — Recommended videos", 420),
        ];
        let (process, title, seconds) = items[(self.step / 5).min(2)];
        self.step += 1;
        Ok(ActivitySnapshot {
            timestamp: Utc::now().to_rfc3339(),
            process_name: process.into(),
            window_title: title.into(),
            idle_seconds: 0,
            active_seconds: seconds,
            ..Default::default()
        })
    }
}
/// Extension point. No screen capture is performed or persisted.
pub fn capture_screenshot_on_demand() -> Result<Vec<u8>, String> {
    Err("Screenshot capture is not implemented in this MVP".into())
}
/// Future UI Automation adapters should return only explicitly requested text.
#[allow(dead_code)]
pub trait AccessibleContext {
    fn foreground_context(&self) -> Result<String, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_process_name() {
        assert_eq!(process_basename(r"C:\Program Files\App\app.exe"), "app.exe");
    }
    #[test]
    fn tracks_contiguous_active_duration() {
        let mut tracker = DurationTracker {
            previous: Some((
                "app".into(),
                "title".into(),
                std::time::Instant::now() - std::time::Duration::from_secs(3),
                std::time::Duration::from_secs(6),
            )),
        };
        assert_eq!(tracker.update("app", "title", 0, 300), 9);
        assert_eq!(tracker.update("other", "title", 0, 300), 0);
    }
    #[test]
    fn idle_resets_duration() {
        let mut tracker = DurationTracker {
            previous: Some((
                "app".into(),
                "title".into(),
                std::time::Instant::now(),
                std::time::Duration::from_secs(6),
            )),
        };
        assert_eq!(tracker.update("app", "title", 300, 300), 0);
    }
}
