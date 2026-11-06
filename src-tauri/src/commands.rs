use crate::{collector, http, models::*, nebius, storage::Storage, tavily};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::Manager;
use tauri::{AppHandle, State};

pub struct Inner {
    pub frontend_events: crate::frontend_events::State,
    pub engine: crate::engine_observability::Telemetry,
    pub agent: crate::agent_runtime::AgentRuntime,
    pub goal_matching: crate::goal_matching::Runtime,
    pub companion: crate::companion::Runtime,
    pub tracking_settings: crate::tracking::TrackingSettings,
    pub activity_state: crate::tracking::StateTracker,
    pub usage: crate::insights::UsageTracker,
    pub window_history: crate::window_history::Recorder,
    pub unplanned_work: crate::unplanned_work::Runtime,
    pub(crate) vision: crate::vision::Runtime,
    pub(crate) review_planning: Option<crate::activity_review::PlanningContext>,
    pub retention_days: u32,
    pub last_cleanup: Instant,
    pub privacy_revision: u64,
    /// The `privacy_revision` at the last change to sharing, privacy, tracking or provider consent.
    /// Goal and chat edits bump `privacy_revision` only, so a pending answer survives them.
    pub last_consent_change: u64,
    pub memory_session_started_at: String,
    pub buddy: crate::buddy::Runtime,
    pub storage: Storage,
    pub collector: Box<dyn collector::ActivityCollector>,
    pub status: Status,
    pub last_analysis: Option<Instant>,
    pub last_nudge: Option<Instant>,
    pub last_error: Option<String>,
}
impl Inner {
    /// Records a consent-affecting change: sharing, privacy, tracking or provider settings.
    pub fn mark_consent_change(&mut self) {
        self.privacy_revision += 1;
        self.companion.invalidate_screen_context();
        self.unplanned_work.clear();
        self.last_consent_change = self.privacy_revision;
        crate::engine_observability::invalidate_connections(self);
    }
    /// Whether consent changed after a request captured `revision`.
    pub fn consent_changed(&self, revision: u64) -> bool {
        self.last_consent_change > revision
    }
}

pub struct AppState {
    pub(crate) screen_context_read: tokio::sync::Mutex<()>,
    pub companion_request: tokio::sync::Mutex<()>,
    pub recommendation: tokio::sync::Mutex<()>,
    pub inner: Mutex<Inner>,
    pub client: reqwest::Client,
    pub analysis: tokio::sync::Mutex<()>,
}
impl AppState {
    pub fn new(mut storage: Storage) -> Result<Self, String> {
        #[cfg(not(test))]
        crate::shared_ai::initialize(&storage)?;
        let demo = http::enabled("DEMO_MODE");
        let mut buddy = crate::buddy::Runtime::new(crate::attention::validate_preferences(
            storage.buddy_preferences()?,
        )?);
        buddy.position = storage.read_setting("buddy_position")?;
        buddy.last_attempt_at = storage.read_setting("buddy_last_attempt")?;
        let retention_days = storage.read_setting("retention_days")?.unwrap_or(0);
        crate::privacy::validate_retention(retention_days)?;
        if retention_days > 0 {
            let cutoff =
                (chrono::Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();
            storage.purge_history(Some(&cutoff))?;
        }
        let tracking = crate::tracking::requested(&storage)?;
        let companion = crate::companion::Runtime::load(&storage)?;
        buddy.view.companion = companion.view.clone();
        buddy.view.completion_revision = storage
            .read_setting("mascot_completion_revision")?
            .unwrap_or(0);
        Ok(Self {
            screen_context_read: tokio::sync::Mutex::new(()),
            companion_request: tokio::sync::Mutex::new(()),
            recommendation: tokio::sync::Mutex::new(()),
            inner: Mutex::new(Inner {
                frontend_events: Default::default(),
                engine: Default::default(),
                agent: crate::agent_runtime::AgentRuntime::load(&storage)?,
                goal_matching: Default::default(),
                companion,
                tracking_settings: storage.tracking_settings()?,
                activity_state: Default::default(),
                usage: Default::default(),
                window_history: Default::default(),
                unplanned_work: Default::default(),
                vision: Default::default(),
                review_planning: None,
                retention_days,
                last_cleanup: Instant::now(),
                privacy_revision: 0,
                last_consent_change: 0,
                memory_session_started_at: chrono::Utc::now().to_rfc3339(),
                buddy,
                storage,
                collector: collector::create(demo),
                status: Status {
                    tracking,
                    tracking_error: None,
                    last_collected_at: None,
                    ai_enabled: false,
                    dnd: false,
                    demo,
                    mock_ai: http::enabled("AI_MOCK"),
                    nebius_configured: http::configured("NEBIUS_API_KEY"),
                    tavily_configured: http::configured("TAVILY_API_KEY"),
                },
                last_analysis: None,
                last_nudge: None,
                last_error: None,
            }),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Could not initialize HTTP client")?,
            analysis: tokio::sync::Mutex::new(()),
        })
    }
}
fn valid_text(text: &str) -> Result<&str, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 500 {
        Err("Enter between 1 and 500 characters".into())
    } else {
        Ok(text)
    }
}

#[tauri::command(async)]
pub fn set_provider_key(
    window: tauri::WebviewWindow,
    state: State<AppState>,
    provider: String,
    key: Option<String>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Open Settings in the main window".into());
    }
    if crate::shared_ai::mode() == crate::shared_ai::ConnectionMode::BuiltIn {
        return Err("Choose Use my API keys before editing personal credentials".into());
    }
    let name = crate::credentials::provider_name(&provider)?;
    let key = key
        .as_deref()
        .map(crate::credentials::validate_key)
        .transpose()?;
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    crate::credentials::write(name, key)?;
    inner.mark_consent_change();
    if key.is_some() {
        inner.storage.connection.execute("UPDATE core_goal_analyses SET state='queued',message=NULL,attempts=0 WHERE state='failed'",[]).map_err(|_| "Could not resume goal suggestions")?;
    }
    inner.status.nebius_configured = http::configured("NEBIUS_API_KEY");
    inner.status.tavily_configured = http::configured("TAVILY_API_KEY");
    inner.last_error = None;
    crate::frontend_events::changed(
        window.app_handle(),
        &[
            crate::frontend_events::Resource::Dashboard,
            crate::frontend_events::Resource::Services,
            crate::frontend_events::Resource::Preferences,
            crate::frontend_events::Resource::Workspace,
        ],
        false,
    );
    Ok(())
}

#[tauri::command(async)]
pub fn get_dashboard(state: State<AppState>) -> Result<Dashboard, String> {
    let inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    let goal = inner.storage.goal()?;
    let activity = if let Some(g) = &goal {
        inner.storage.recent(g.id)?
    } else {
        vec![]
    };
    let decision = if let Some(g) = &goal {
        inner.storage.latest_decision(g.id)?
    } else {
        None
    };
    Ok(Dashboard {
        work_state: crate::work_state::compute(&inner)?,
        goal_matching: inner.goal_matching.view.clone(),
        user_settings: inner.storage.user_settings()?,
        app_rules: goal
            .as_ref()
            .map(|g| inner.storage.app_rules(g.id))
            .transpose()?
            .unwrap_or_default(),
        today: inner.storage.today(goal.as_ref().map(|g| g.id))?,
        goal_plan: goal
            .as_ref()
            .map(|g| inner.storage.goal_plan(g.id))
            .transpose()?,
        saved_goals: inner.storage.saved_goals()?,
        retention_days: inner.retention_days,
        recommendations: inner.storage.recommendation_history()?,
        version: env!("CARGO_PKG_VERSION"),
        buddy: inner.buddy.view.clone(),
        goal,
        activity,
        decision,
        status: inner.status.clone(),
        last_error: inner.last_error.clone(),
    })
}
#[tauri::command(async)]
pub fn set_goal(text: String, app: AppHandle, state: State<AppState>) -> Result<Goal, String> {
    let text = valid_text(&text)?;
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    let goal = inner.storage.set_goal(text)?;
    inner.status.tracking = inner.storage.user_settings()?.onboarding.tracking_consent;
    inner.last_analysis = None;
    inner.last_nudge = None;
    inner.last_error = None;
    inner.collector = collector::create(inner.status.demo);
    inner.usage = Default::default();
    inner.activity_state.stop(false);
    inner.buddy.foreground = None;
    inner.buddy.clear();
    crate::buddy::sync(&app, &mut inner)?;
    crate::companion::emit(&app, "goal.created", &goal);
    Ok(goal)
}
#[tauri::command(async)]
pub fn get_current_goal(state: State<AppState>) -> Result<Option<Goal>, String> {
    state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?
        .storage
        .goal()
}
#[tauri::command(async)]
pub fn get_recent_activity(state: State<AppState>) -> Result<Vec<ActivitySnapshot>, String> {
    let inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if let Some(goal) = inner.storage.goal()? {
        inner.storage.recent(goal.id)
    } else {
        Ok(vec![])
    }
}
#[tauri::command(async)]
pub fn set_tracking(enabled: bool, app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if enabled {
        let mut settings = inner.storage.user_settings()?;
        if !settings.onboarding.completed {
            return Err("Finish onboarding before enabling activity tracking".into());
        }
        settings.onboarding.tracking_consent = true;
        inner.storage.write_setting("user_settings", &settings)?;
    }
    inner
        .storage
        .write_setting("tracking_requested", &enabled)?;
    inner.status.tracking = enabled;
    if !enabled {
        inner.agent.suspend();
    }
    inner.goal_matching.clear();
    inner.status.tracking_error = None;
    if !enabled {
        if let Some(goal) = inner.storage.goal()? {
            if inner.activity_state.state != crate::tracking::ActivityState::Paused {
                inner
                    .storage
                    .tracking_event(goal.id, crate::tracking::ActivityEvent::Paused)?;
            }
        }
    }
    inner.activity_state.stop(false);
    inner.buddy.clear();
    inner.collector = collector::create(inner.status.demo);
    inner.usage = Default::default();
    inner.buddy.foreground = None;
    crate::buddy::sync(&app, &mut inner)?;
    Ok(())
}
#[tauri::command(async)]
pub fn set_ai_enabled(enabled: bool, app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.status.ai_enabled = enabled;
    inner.last_error = None;
    if !enabled {
        inner.buddy.clear();
        crate::buddy::sync(&app, &mut inner)?;
    }
    Ok(())
}
#[tauri::command(async)]
pub fn set_dnd(enabled: bool, app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.status.dnd = enabled;
    if enabled {
        inner.agent.suspend();
    }
    inner.buddy.clear();
    crate::buddy::sync(&app, &mut inner)?;
    Ok(())
}
#[tauri::command(async)]
pub fn get_activity_snapshot(state: State<AppState>) -> Result<ActivitySnapshot, String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if !inner.status.tracking {
        return Err("Activity tracking is paused".into());
    }
    inner.collector.collect()
}
#[tauri::command(async)]
pub fn capture_screenshot_on_demand() -> Result<Vec<u8>, String> {
    collector::capture_screenshot_on_demand()
}
#[tauri::command(async)]
pub fn dismiss_buddy(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.last_nudge = Some(Instant::now());
    inner.buddy.clear();
    crate::buddy::sync(&app, &mut inner)?;
    Ok(())
}
#[tauri::command(async)]
pub fn save_feedback(
    decision_id: i64,
    related: bool,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.storage.feedback(decision_id, related)?;
    inner.last_nudge = Some(Instant::now());
    inner.buddy.clear();
    crate::buddy::sync(&app, &mut inner)?;
    Ok(())
}
#[tauri::command(async)]
pub async fn search_web(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    tavily::tavily_search(&state.client, valid_text(&query)?).await
}
#[tauri::command(async)]
pub async fn analyze_focus(app: AppHandle, state: State<'_, AppState>) -> Result<Decision, String> {
    analyze(&app, &state, false).await
}

pub fn collect(state: &AppState) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if !inner.status.tracking {
        return Ok(());
    }
    {
        let settings = inner.tracking_settings.clone();
        let excluded = inner.buddy.view.preferences.excluded_apps.clone();
        inner.collector.configure(&settings, &excluded);
        let mut snapshot = match inner.collector.collect() {
            Ok(snapshot) => {
                inner.status.tracking_error = None;
                inner.status.last_collected_at = Some(snapshot.timestamp.clone());
                snapshot
            }
            Err(error) => {
                // A title that changed mid-read is a skipped sample, not a lost context:
                // the trackers' own 15-second gap rule decides whether continuity holds.
                if error == collector::CONTEXT_CHANGED {
                    return Ok(());
                }
                inner.usage = Default::default();
                inner.window_history.clear();
                inner.unplanned_work.reset_sample();
                inner.companion.semantic.invalidate_evidence();
                inner.status.tracking_error = Some(error.clone());
                return Err(error);
            }
        };
        inner.buddy.fullscreen = !inner.status.demo && collector::foreground_fullscreen();
        let in_sumi = [
            "sumi.exe",
            "sumi",
            "desktop-buddy.exe",
            "desktop-buddy",
            "Desktop Buddy",
        ]
        .iter()
        .any(|name| snapshot.process_name.eq_ignore_ascii_case(name));
        let planning = inner
            .review_planning
            .as_ref()
            .filter(|p| p.at.elapsed() < Duration::from_secs(30));
        let planning_goal = planning.and_then(|p| p.goal_id);
        let allowed = (!in_sumi || planning.is_some())
            && !inner.companion.view.preferences.paused
            && !crate::attention::excluded(&inner.buddy.view.preferences, &snapshot.process_name)
            && !crate::companion::sensitive_window(&snapshot);
        if in_sumi {
            snapshot.process_name = "desktop-buddy".into();
            snapshot.window_title = planning_goal
                .map(|id| format!("Planning goal {id} in sumi"))
                .unwrap_or_else(|| "Planning in sumi".into());
            snapshot.browser = None;
        }
        if !inner.tracking_settings.browser_metadata {
            snapshot.browser =
                crate::browser::context(&snapshot.process_name, &snapshot.window_title, None);
        }
        if allowed && !in_sumi {
            crate::semantic_observation::record_metadata(&mut inner, &snapshot);
        } else {
            inner.companion.semantic.invalidate_evidence();
        }
        crate::goal_matching::observe_inner(&mut inner, &snapshot, allowed && !in_sumi)?;
        inner.buddy.foreground = (allowed && !in_sumi).then(|| snapshot.clone());
        crate::companion::observe(&mut inner, &snapshot, allowed && !in_sumi);
        let selected_goal = if in_sumi {
            planning_goal
        } else {
            inner.storage.goal()?.map(|g| g.id)
        };
        let crate::commands::Inner {
            window_history,
            storage,
            ..
        } = &mut *inner;
        let changed = window_history.observe(
            storage,
            &snapshot,
            selected_goal,
            allowed,
            settings.idle_seconds,
        )?;
        if changed {
            crate::engine_observability::record(
                &mut inner,
                "observe",
                "foreground_changed",
                "observed",
                None,
                selected_goal,
            );
        }
        let goal = if in_sumi { None } else { inner.storage.goal()? };
        let goal_id = selected_goal;
        let revision = inner.activity_state.revision;
        let activity_state =
            inner
                .activity_state
                .sample(goal_id.unwrap_or(0), &snapshot, allowed, &settings);
        if let Some(goal) = goal
            .as_ref()
            .filter(|_| allowed && inner.activity_state.revision != revision)
        {
            inner
                .storage
                .tracking_event(goal.id, inner.activity_state.event)?;
        }
        let mut observed_interval = None;
        if let Some(interval) =
            inner
                .usage
                .observe(goal_id.unwrap_or(0), &snapshot, allowed, activity_state)
        {
            if inner.storage.review_interval_available(&interval)? {
                let attribution = if in_sumi {
                    crate::relevance::ActivityMatch {
                        goal_id: planning_goal.map(|id| id.to_string()),
                        confidence: if planning_goal.is_some() { 1. } else { 0. },
                        reason: "Explicit in-app planning context".into(),
                    }
                } else {
                    crate::goal_matching::attribution(&inner, &snapshot)?
                };
                if let Some(goal) = &goal {
                    inner.storage.record_interval_with_match(
                        goal.id,
                        &interval,
                        Some(crate::relevance::ActivityMatch {
                            goal_id: attribution.goal_id.clone(),
                            confidence: attribution.confidence,
                            reason: attribution.reason.clone(),
                        }),
                    )?;
                }
                inner
                    .storage
                    .record_review_interval(&interval, &attribution)?;
                observed_interval = Some(interval);
            }
        }
        crate::unplanned_work::observe(
            &mut inner,
            &snapshot,
            allowed && !in_sumi,
            goal_id.zip(observed_interval.as_ref()),
        )?;
        inner.buddy.foreground = (allowed && !in_sumi).then(|| snapshot.clone());
        if let Some(goal) = goal.as_ref().filter(|_| allowed) {
            inner.storage.activity(goal.id, &snapshot)?;
        }
    }
    Ok(())
}
/// Automatic focus warnings are retired; explicit focus analysis remains available.
pub fn should_analyze(_state: &AppState) -> bool {
    false
}
async fn analyze(app: &AppHandle, state: &AppState, automatic: bool) -> Result<Decision, String> {
    let _guard = state
        .analysis
        .try_lock()
        .map_err(|_| "A focus check is already running")?;
    let (goal, context, activity, mock, privacy_revision) = {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "Application state unavailable")?;
        if !inner.status.ai_enabled {
            return Err("Enable AI check-ins before sending activity to Nebius".into());
        }
        if automatic && !inner.status.tracking {
            return Err("Activity tracking is paused".into());
        }
        let goal = inner.storage.goal()?.ok_or("Set a goal first")?;
        let activity: Vec<_> = inner
            .storage
            .recent(goal.id)?
            .into_iter()
            .filter(|a| !crate::attention::excluded(&inner.buddy.view.preferences, &a.process_name))
            .collect();
        inner.last_analysis = Some(Instant::now());
        if activity.is_empty() {
            return Err("Collect some activity before checking focus".into());
        }
        let context = inner.storage.goal_context(&goal)?;
        (
            goal,
            context,
            activity,
            inner.status.mock_ai,
            inner.privacy_revision,
        )
    };
    let result = if mock {
        Ok(nebius::mock(&activity))
    } else {
        crate::engine_observability::observe_specialist(
            state,
            "focus_check",
            nebius::analyze(&state.client, &context, &activity),
        )
        .await
    };
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if !inner.status.ai_enabled
        || inner.storage.goal()?.map(|g| g.id) != Some(goal.id)
        || inner.privacy_revision != privacy_revision
    {
        return Err("Focus check discarded because the goal or consent changed".into());
    }
    if automatic
        && (inner.buddy.view.quiet_reason.is_some() || inner.status.dnd || !inner.status.tracking)
    {
        return Err("Focus check discarded while sumi is quiet".into());
    }
    let mut decision = match result {
        Ok(d) => d,
        Err(error) => {
            let message = crate::goal_analysis::user_error("focus_check", &error);
            inner.last_error = Some(message.clone());
            return Err(message);
        }
    };
    inner.last_error = None;
    inner.storage.decision(goal.id, &mut decision)?;
    let cooldown = inner
        .last_nudge
        .map(|t| t.elapsed() >= Duration::from_secs(600))
        .unwrap_or(true);
    if inner.buddy.view.quiet_reason.is_none()
        && crate::tracking::notifications_allowed(&inner)
        && crate::insights::explicit_category(&inner)?.is_none()
        && should_nudge(
            &inner.status,
            cooldown,
            &decision,
            activity.last(),
            inner.tracking_settings.idle_seconds,
        )
        && inner.buddy.shown.is_none()
    {
        inner.buddy.decision(decision.clone());
        crate::buddy::sync(app, &mut inner)?;
        inner.last_nudge = Some(Instant::now());
    }
    Ok(decision)
}
pub async fn automatic_analysis(app: &AppHandle, state: &AppState) {
    let _ = analyze(app, state, true).await;
}

fn should_nudge(
    status: &Status,
    cooldown: bool,
    decision: &Decision,
    latest: Option<&ActivitySnapshot>,
    idle_threshold: u64,
) -> bool {
    status.tracking
        && status.ai_enabled
        && !status.dnd
        && cooldown
        && decision.confidence >= 0.75
        && decision.state != FocusState::Focused
        && decision.action != Action::Wait
        && latest
            .map(|a| {
                a.idle_seconds < idle_threshold
                    && (decision.action == Action::OfferHelp || a.active_seconds >= 120)
            })
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_records_unassigned_windows_on_both_platform_contexts() {
        struct Fixture(ActivitySnapshot);
        impl collector::ActivityCollector for Fixture {
            fn collect(&mut self) -> Result<ActivitySnapshot, String> {
                Ok(self.0.clone())
            }
        }
        for process in ["Editor.app", "editor.exe"] {
            let state =
                AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
            {
                let mut inner = state.inner.lock().unwrap();
                inner.status.tracking = true;
                inner.status.demo = true;
                inner.collector = Box::new(Fixture(ActivitySnapshot {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    process_name: process.into(),
                    window_title: "First window".into(),
                    window_id: Some(1),
                    ..Default::default()
                }));
            }
            collect(&state).unwrap();
            {
                let mut inner = state.inner.lock().unwrap();
                assert!(inner.storage.goal().unwrap().is_none());
                assert_eq!(
                    inner
                        .storage
                        .connection
                        .query_row(
                            "SELECT count(*) FROM window_activity WHERE goal_id IS NULL",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    1
                );
                inner.buddy.view.preferences.excluded_apps = vec![process.into()];
            }
            collect(&state).unwrap();
            assert_eq!(
                state
                    .inner
                    .lock()
                    .unwrap()
                    .storage
                    .connection
                    .query_row("SELECT count(*) FROM window_activity", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
    #[test]
    fn nudges_respect_consent_pause_dnd_idle_and_confidence() {
        let mut status = Status {
            tracking: true,
            tracking_error: None,
            last_collected_at: None,
            ai_enabled: true,
            dnd: false,
            demo: false,
            mock_ai: false,
            nebius_configured: true,
            tavily_configured: true,
        };
        let mut decision = Decision {
            id: None,
            state: FocusState::Drifting,
            confidence: 0.9,
            reason: "Unrelated videos".into(),
            action: Action::Intervene,
        };
        let mut activity = ActivitySnapshot {
            timestamp: String::new(),
            process_name: "browser".into(),
            window_title: "Videos".into(),
            idle_seconds: 0,
            active_seconds: 180,
            ..Default::default()
        };
        assert!(should_nudge(&status, true, &decision, Some(&activity), 300));
        assert!(!should_nudge(
            &status,
            false,
            &decision,
            Some(&activity),
            300
        ));
        status.dnd = true;
        assert!(!should_nudge(
            &status,
            true,
            &decision,
            Some(&activity),
            300
        ));
        status.dnd = false;
        status.tracking = false;
        assert!(!should_nudge(
            &status,
            true,
            &decision,
            Some(&activity),
            300
        ));
        status.tracking = true;
        status.ai_enabled = false;
        assert!(!should_nudge(
            &status,
            true,
            &decision,
            Some(&activity),
            300
        ));
        status.ai_enabled = true;
        activity.idle_seconds = 400;
        assert!(should_nudge(&status, true, &decision, Some(&activity), 600));
        assert!(!should_nudge(
            &status,
            true,
            &decision,
            Some(&activity),
            300
        ));
        activity.idle_seconds = 300;
        assert!(!should_nudge(
            &status,
            true,
            &decision,
            Some(&activity),
            300
        ));
        activity.idle_seconds = 0;
        activity.active_seconds = 15;
        assert!(!should_nudge(
            &status,
            true,
            &decision,
            Some(&activity),
            300
        ));
        activity.active_seconds = 180;
        decision.confidence = 0.4;
        assert!(!should_nudge(
            &status,
            true,
            &decision,
            Some(&activity),
            300
        ));
    }
    #[test]
    fn rejects_empty_and_oversized_input() {
        assert!(valid_text("  ").is_err());
        assert!(valid_text(&"a".repeat(501)).is_err());
        assert_eq!(valid_text("  Build a demo  ").unwrap(), "Build a demo");
    }

    #[test]
    fn demo_runs_from_goal_through_feedback() {
        let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
        let goal = storage
            .set_goal("Finish the hackathon presentation")
            .unwrap();
        let mut collector = collector::create(true);
        for _ in 0..15 {
            storage
                .activity(goal.id, &collector.collect().unwrap())
                .unwrap();
        }
        let activity = storage.recent(goal.id).unwrap();
        assert_eq!(activity.len(), 3);
        assert_eq!(
            activity
                .iter()
                .map(|a| a.active_seconds)
                .collect::<Vec<_>>(),
            vec![600, 300, 420]
        );
        let mut decision = nebius::mock(&activity);
        assert_eq!(decision.state, FocusState::Drifting);
        storage.decision(goal.id, &mut decision).unwrap();
        storage.feedback(decision.id.unwrap(), true).unwrap();
        let saved = storage.latest_decision(goal.id).unwrap().unwrap();
        assert_eq!(saved.id, decision.id);
        let feedback_count: i64 = storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM feedback WHERE decision_id=?1 AND related=1",
                [saved.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(feedback_count, 1);
    }
}

#[tauri::command(async)]
pub fn get_accessibility_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos::get_accessibility_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
#[tauri::command(async)]
pub fn request_accessibility_permission(window: tauri::WebviewWindow) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::request_accessibility_permission(window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok(true)
    }
}

#[tauri::command(async)]
pub fn reset_accessibility_permission(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::reset_accessibility_permission(&app, window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, window);
        Ok(true)
    }
}

#[cfg(test)]
mod collection_status_tests {
    use super::*;

    struct Sample(Result<ActivitySnapshot, String>);
    impl crate::collector::ActivityCollector for Sample {
        fn collect(&mut self) -> Result<ActivitySnapshot, String> {
            self.0.clone()
        }
    }

    #[test]
    fn heartbeat_records_successful_collection_without_claiming_goal_time() {
        let state =
            AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
        let timestamp = chrono::Utc::now().to_rfc3339();
        {
            let mut inner = state.inner.lock().unwrap();
            inner.status.tracking = true;
            inner.status.demo = true;
            inner.collector = Box::new(Sample(Ok(ActivitySnapshot {
                timestamp: timestamp.clone(),
                process_name: "work-app".into(),
                ..Default::default()
            })));
        }
        collect(&state).unwrap();
        let inner = state.inner.lock().unwrap();
        assert_eq!(inner.status.last_collected_at.as_ref(), Some(&timestamp));
        assert!(inner.storage.goal().unwrap().is_none());
        assert!(inner.storage.today(None).unwrap().apps.is_empty());
    }

    #[test]
    fn failed_collection_does_not_advance_the_heartbeat() {
        let state =
            AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
        {
            let mut inner = state.inner.lock().unwrap();
            inner.status.tracking = true;
            inner.collector = Box::new(Sample(Err("Permission required".into())));
        }
        assert!(collect(&state).is_err());
        let inner = state.inner.lock().unwrap();
        assert!(inner.status.last_collected_at.is_none());
        assert_eq!(
            inner.status.tracking_error.as_deref(),
            Some("Permission required")
        );
    }
}
