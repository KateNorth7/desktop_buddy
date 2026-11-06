use crate::{models::ActivitySnapshot, storage::Storage};
use chrono::Timelike;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::State;
pub const DEFAULT_IDLE_SECONDS: u64 = 300;
pub fn configure_goal_monitoring(
    inner: &mut crate::commands::Inner,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = inner.storage.user_settings()?;
    if !settings.onboarding.completed {
        return Err("Finish setup before enabling goal tracking".into());
    }
    if enabled {
        settings.onboarding.tracking_consent = true;
        let mut ai = inner.storage.ai_preferences()?;
        ai.enabled = true;
        ai.automatic_goal_matching = ai.share_goal_context;
        inner
            .storage
            .write_setting("ai_assistance_preferences", &ai)?;
    }
    inner.storage.write_setting("user_settings", &settings)?;
    inner
        .storage
        .write_setting("tracking_requested", &enabled)?;
    let mut companion = inner.companion.view.preferences.clone();
    companion.screen_task_detection = enabled;
    if enabled {
        companion.paused = false;
    }
    inner
        .storage
        .write_setting("companion_preferences", &companion)?;
    inner.companion.view.preferences = companion;
    inner.status.tracking = enabled;
    inner.status.tracking_error = None;
    inner.collector = crate::collector::create(inner.status.demo);
    inner.usage = Default::default();
    inner.window_history.clear();
    inner.activity_state.stop(false);
    inner.goal_matching.clear();
    inner.companion.invalidate();
    inner.buddy.foreground = None;
    inner.buddy.clear();
    inner.mark_consent_change();
    Ok(())
}
#[tauri::command(async)]
pub fn set_goal_monitoring(
    enabled: bool,
    app: tauri::AppHandle,
    state: tauri::State<crate::commands::AppState>,
) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    configure_goal_monitoring(&mut inner, enabled)?;
    crate::buddy::sync(&app, &mut inner)
}
pub fn requested(storage: &crate::storage::Storage) -> Result<bool, String> {
    let settings = storage.user_settings()?;
    Ok(settings.onboarding.completed
        && settings.onboarding.tracking_consent
        && storage
            .read_setting::<bool>("tracking_requested")?
            .unwrap_or(true))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackingSettings {
    pub idle_seconds: u64,
    pub drifting_context_seconds: u64,
    pub drifting_no_input_seconds: u64,
    pub working_start_minute: u32,
    pub working_end_minute: u32,
    pub browser_metadata: bool,
}
impl Default for TrackingSettings {
    fn default() -> Self {
        Self {
            idle_seconds: DEFAULT_IDLE_SECONDS,
            drifting_context_seconds: 180,
            drifting_no_input_seconds: 120,
            working_start_minute: 540,
            working_end_minute: 1080,
            browser_metadata: true,
        }
    }
}
impl TrackingSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(60..=3600).contains(&self.idle_seconds)
            || self.drifting_no_input_seconds < 30
            || self.drifting_no_input_seconds >= self.idle_seconds
            || self.drifting_context_seconds < self.drifting_no_input_seconds
            || self.drifting_context_seconds > 86400
            || self.working_start_minute >= 1440
            || self.working_end_minute >= 1440
            || self.working_start_minute == self.working_end_minute
        {
            return Err("Invalid idle, drifting or working-hours settings".into());
        }
        Ok(())
    }
    pub fn working_at(&self, minute: u32) -> bool {
        if self.working_start_minute < self.working_end_minute {
            minute >= self.working_start_minute && minute < self.working_end_minute
        } else {
            minute >= self.working_start_minute || minute < self.working_end_minute
        }
    }
    pub fn working_now(&self) -> bool {
        let now = chrono::Local::now();
        self.working_at(now.hour() * 60 + now.minute())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Focused,
    #[default]
    Paused,
    Drifting,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEvent {
    Working,
    #[default]
    Paused,
    Drifting,
    Resumed,
    GoalCompleted,
}

/// Equality is deliberately conservative: unknown browser addresses cannot
/// inherit a previously observed domain or bridge two different tabs.
pub fn same_context(a: &ActivitySnapshot, b: &ActivitySnapshot) -> bool {
    a.process_name.eq_ignore_ascii_case(&b.process_name)
        && a.window_id == b.window_id
        && a.window_title == b.window_title
        && a.browser == b.browser
        && a.document_key == b.document_key
}

#[derive(Default)]
pub struct StateTracker {
    context: Option<(i64, ActivitySnapshot, Instant, Instant)>,
    pub state: ActivityState,
    pub event: ActivityEvent,
    pub revision: u64,
}
impl StateTracker {
    fn transition(&mut self, state: ActivityState, event: ActivityEvent) {
        if self.state != state || self.event != event {
            self.state = state;
            self.event = event;
            self.revision += 1;
        }
    }
    pub fn stop(&mut self, completed: bool) {
        self.context = None;
        self.transition(
            ActivityState::Paused,
            if completed {
                ActivityEvent::GoalCompleted
            } else {
                ActivityEvent::Paused
            },
        );
    }
    pub fn sample(
        &mut self,
        goal: i64,
        a: &ActivitySnapshot,
        allowed: bool,
        settings: &TrackingSettings,
    ) -> ActivityState {
        self.sample_at(goal, a, allowed, settings, Instant::now())
    }
    pub(crate) fn sample_at(
        &mut self,
        goal: i64,
        a: &ActivitySnapshot,
        allowed: bool,
        settings: &TrackingSettings,
        now: Instant,
    ) -> ActivityState {
        if !allowed {
            self.stop(false);
            return self.state;
        }
        let since = self
            .context
            .as_ref()
            .filter(|(g, previous, _, last)| {
                *g == goal
                    && same_context(previous, a)
                    && now.duration_since(*last) <= Duration::from_secs(15)
            })
            .map(|(_, _, since, _)| *since)
            .unwrap_or(now);
        let state = if !a.engaged(settings.idle_seconds) {
            ActivityState::Paused
        } else if !a.media_playing
            && a.idle_seconds >= settings.drifting_no_input_seconds
            && now.duration_since(since).as_secs() >= settings.drifting_context_seconds
        {
            ActivityState::Drifting
        } else {
            ActivityState::Focused
        };
        if state != self.state {
            let event = match state {
                ActivityState::Focused if self.revision > 0 => ActivityEvent::Resumed,
                ActivityState::Focused => ActivityEvent::Working,
                ActivityState::Paused => ActivityEvent::Paused,
                ActivityState::Drifting => ActivityEvent::Drifting,
            };
            self.transition(state, event);
        }
        self.context = Some((goal, a.clone(), since, now));
        self.state
    }
}

pub fn notifications_allowed(inner: &crate::commands::Inner) -> bool {
    !crate::companion::quiet_mode(inner)
        && inner.activity_state.state != ActivityState::Paused
        && !inner
            .buddy
            .foreground
            .as_ref()
            .is_some_and(|a| a.media_playing)
}

#[tauri::command(async)]
pub fn get_tracking_settings(
    state: State<crate::commands::AppState>,
) -> Result<TrackingSettings, String> {
    Ok(state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?
        .tracking_settings
        .clone())
}
#[tauri::command(async)]
pub fn set_tracking_settings(
    settings: TrackingSettings,
    app: tauri::AppHandle,
    state: State<crate::commands::AppState>,
) -> Result<(), String> {
    settings.validate()?;
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner
        .storage
        .write_setting("tracking_settings", &settings)?;
    inner.tracking_settings = settings;
    inner.mark_consent_change();
    inner.usage = Default::default();
    inner.window_history.clear();
    inner.activity_state.stop(false);
    inner.buddy.clear();
    crate::buddy::sync(&app, &mut inner)
}

impl Storage {
    pub fn tracking_settings(&self) -> Result<TrackingSettings, String> {
        let settings: TrackingSettings =
            self.read_setting("tracking_settings")?.unwrap_or_default();
        settings.validate()?;
        Ok(settings)
    }
}

#[cfg(test)]
pub(crate) fn allow_notifications(inner: &mut crate::commands::Inner) {
    let mut preferences: crate::core::CorePreferences = inner
        .storage
        .read_setting("core_preferences")
        .unwrap()
        .unwrap_or_default();
    preferences.working_days = (0..7).collect();
    inner
        .storage
        .write_setting("core_preferences", &preferences)
        .unwrap();
    let now = chrono::Local::now();
    inner.tracking_settings.working_start_minute = now.hour() * 60 + now.minute();
    inner.tracking_settings.working_end_minute =
        (inner.tracking_settings.working_start_minute + 60) % 1440;
    inner.activity_state.state = ActivityState::Focused;
}

#[cfg(test)]
#[path = "tracking_tests.rs"]
mod tests;
