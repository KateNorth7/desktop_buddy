use super::{process_basename, ActivityCollector, DurationTracker};
use crate::browser::BrowserActivityProvider;
use crate::models::ActivitySnapshot;
use chrono::Utc;
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::CloseHandle,
        System::{
            SystemInformation::GetTickCount,
            Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId},
        },
    },
};

#[derive(Default)]
pub struct WindowsCollector {
    tracker: DurationTracker,
    previous_window: usize,
    browser: super::browser_windows::WindowsBrowserProvider,
    settings: crate::tracking::TrackingSettings,
    excluded: Vec<String>,
}
impl ActivityCollector for WindowsCollector {
    fn configure(&mut self, settings: &crate::tracking::TrackingSettings, excluded: &[String]) {
        self.settings = settings.clone();
        self.excluded = excluded.to_vec();
    }
    fn collect(&mut self) -> Result<ActivitySnapshot, String> {
        // Handles and buffers remain local; the process handle is closed before returning.
        unsafe {
            if !session_active() {
                self.tracker = DurationTracker::default();
                return Err("Desktop session is inactive or locked".into());
            }
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return Err("No foreground window is available".into());
            }
            if self.previous_window != hwnd.0 as usize {
                self.tracker = DurationTracker::default();
                self.previous_window = hwnd.0 as usize;
            }
            let mut title = [0u16; 1024];
            let title_len = GetWindowTextW(hwnd, &mut title);
            let window_title: String =
                String::from_utf16_lossy(&title[..title_len.max(0) as usize])
                    .chars()
                    .take(160)
                    .collect();
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let process_name =
                if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let mut path = [0u16; 32768];
                    let mut size = path.len() as u32;
                    let result = QueryFullProcessImageNameW(
                        handle,
                        PROCESS_NAME_WIN32,
                        PWSTR(path.as_mut_ptr()),
                        &mut size,
                    );
                    let _ = CloseHandle(handle);
                    if result.is_ok() {
                        process_basename(&String::from_utf16_lossy(&path[..size as usize]))
                    } else {
                        "Protected process".into()
                    }
                } else {
                    "Protected process".into()
                };
            let mut last = LASTINPUTINFO {
                cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            if !GetLastInputInfo(&mut last).as_bool() {
                return Err("Could not read idle time".into());
            }
            let idle_seconds = GetTickCount().wrapping_sub(last.dwTime) as u64 / 1000;
            let (domain, media_playing) = if !self
                .excluded
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&process_name))
            {
                self.browser.metadata(
                    if self.settings.browser_metadata {
                        hwnd.0 as usize
                    } else {
                        0
                    },
                    &process_name,
                    &window_title,
                )
            } else {
                (None, false)
            };
            let browser = crate::browser::context(&process_name, &window_title, domain);
            let mut latest_title = [0u16; 1024];
            let len = GetWindowTextW(hwnd, &mut latest_title);
            let latest: String = String::from_utf16_lossy(&latest_title[..len.max(0) as usize])
                .chars()
                .take(160)
                .collect();
            if GetForegroundWindow() != hwnd || latest != window_title {
                self.tracker = DurationTracker::default();
                return Err(super::CONTEXT_CHANGED.into());
            }
            let context_key = format!(
                "{}|{}",
                window_title,
                browser
                    .as_ref()
                    .and_then(|b| b.domain.as_deref())
                    .unwrap_or("")
            );
            let active_seconds = self.tracker.update(
                &process_name,
                &context_key,
                idle_seconds,
                self.settings.idle_seconds,
            );
            let window_title = browser
                .as_ref()
                .map(|b| b.page_title.clone())
                .unwrap_or(window_title);
            Ok(ActivitySnapshot {
                timestamp: Utc::now().to_rfc3339(),
                process_name,
                window_title,
                idle_seconds,
                active_seconds,
                window_id: Some(hwnd.0 as u64),
                browser,
                media_playing,
                document_key: None,
            })
        }
    }
}

pub fn foreground_fullscreen() -> bool {
    use windows::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        UI::WindowsAndMessaging::{GetDesktopWindow, GetShellWindow, GetWindowRect},
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() || window == GetDesktopWindow() || window == GetShellWindow() {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }
        rect.left <= info.rcMonitor.left
            && rect.top <= info.rcMonitor.top
            && rect.right >= info.rcMonitor.right
            && rect.bottom >= info.rcMonitor.bottom
    }
}
/// Only a boolean safety guard is returned; nothing is recorded while tracking is off.
pub fn foreground_idle_seconds() -> Option<u64> {
    unsafe {
        let mut last = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        GetLastInputInfo(&mut last)
            .as_bool()
            .then(|| GetTickCount().wrapping_sub(last.dwTime) as u64 / 1000)
    }
}
/// Only a boolean safety guard is returned; nothing is recorded while tracking is off.
pub fn foreground_meeting() -> bool {
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return false;
        }
        let mut title = [0u16; 512];
        let length = GetWindowTextW(window, &mut title).max(0) as usize;
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        let mut process = String::new();
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut path = [0u16; 32768];
            let mut size = path.len() as u32;
            if QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut size,
            )
            .is_ok()
            {
                process = process_basename(&String::from_utf16_lossy(&path[..size as usize]));
            }
            let _ = CloseHandle(handle);
        }
        let title = String::from_utf16_lossy(&title[..length]);
        crate::attention::meeting(&ActivitySnapshot {
            process_name: process,
            window_title: title.clone(),
            ..Default::default()
        }) || title.to_ascii_lowercase().contains("powerpoint slide show")
    }
}

/// A locked workstation/UAC switches away from the ordinary input desktop.
pub(super) fn session_active() -> bool {
    use windows::Win32::System::StationsAndDesktops::*;
    unsafe {
        let Ok(desktop) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS)
        else {
            return false;
        };
        let mut name = [0u16; 256];
        let result = GetUserObjectInformationW(
            windows::Win32::Foundation::HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            std::mem::size_of_val(&name) as u32,
            None,
        );
        let _ = CloseDesktop(desktop);
        result.is_ok()
            && String::from_utf16_lossy(
                &name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())],
            )
            .eq_ignore_ascii_case("default")
    }
}
