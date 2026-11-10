//! One dispatcher for local observation, bounded reasoning and existing specialist services.
mod check;
mod observe;
pub(crate) use check::hero_stuck;
pub use check::run as headless_background_check;
pub mod reasoning;
#[cfg(test)]
mod tests;
use crate::{
    commands::{AppState, Inner},
    goal_analysis::Service,
    skills,
    storage::Storage,
};
use chrono::{Local, Timelike, Utc};
pub use reasoning::{Decision, Reasoning};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use tauri::{AppHandle, Manager};
use tokio::sync::watch;
const DEBOUNCE: i64 = 5;
const COOLDOWN: i64 = 300;
const EVENT_TTL: i64 = 300;
const MAX_PENDING: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    UserMessage,
    VoiceMessage,
    GoalChanged,
    StepChanged,
    CompletionConfirmed,
    MeaningfulContextChanged,
    WorkContextParked,
    WorkContextResumed,
    PossibleStuck,
    PossibleDrift,
    SessionStarted,
    SessionEnded,
    DayStarted,
    DayEnded,
}
impl EventKind {
    pub fn explicit(self) -> bool {
        matches!(self, Self::UserMessage | Self::VoiceMessage)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    User,
    Domain,
    Collector,
    Runtime,
}
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub kind: EventKind,
    pub goal_id: Option<i64>,
    pub step_id: Option<String>,
    pub at: i64,
    #[serde(skip)]
    pub origin: Origin,
}
impl Event {
    pub fn new(kind: EventKind, goal_id: Option<i64>, at: i64) -> Self {
        Self {
            kind,
            goal_id,
            step_id: None,
            at,
            origin: if kind.explicit() {
                Origin::User
            } else {
                Origin::Domain
            },
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Persisted {
    last_attempt: i64,
}
#[derive(Default, Serialize)]
pub struct Diagnostics {
    pub ai_calls: u64,
    pub suppressed: u64,
    pub cancelled: u64,
    pub verified: u64,
    pub last_decision: Option<Decision>,
    pub last_input: Option<EventKind>,
    pub last_event: Option<EventKind>,
    pub suppression_reason: Option<&'static str>,
    pub last_failure: Option<&'static str>,
}
struct Flight {
    generation: u64,
    privacy: u64,
    data: Value,
    conversation: i64,
    cancel: watch::Sender<bool>,
}
pub struct AgentRuntime {
    pending: VecDeque<Event>,
    generation: u64,
    flight: Option<Flight>,
    saved: Persisted,
    observation: observe::Observation,
    pub diagnostics: Diagnostics,
}
impl AgentRuntime {
    pub fn load(storage: &Storage) -> Result<Self, String> {
        let mut saved: Persisted = storage
            .read_setting("agent_runtime_state")?
            .unwrap_or_default();
        saved.last_attempt = saved.last_attempt.clamp(0, Utc::now().timestamp());
        Ok(Self {
            pending: VecDeque::new(),
            generation: 0,
            flight: None,
            saved,
            observation: Default::default(),
            diagnostics: Default::default(),
        })
    }
    pub fn is_thinking(&self) -> bool {
        self.flight.is_some()
    }
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(flight) = self.flight.take() {
            let _ = flight.cancel.send(true);
            self.diagnostics.cancelled = self.diagnostics.cancelled.saturating_add(1);
        }
    }
    pub fn suspend(&mut self) {
        self.cancel();
        self.pending.clear();
    }
    pub fn enqueue(&mut self, event: Event) {
        // Runtime effects and memory writes are terminal: never feed them back into reasoning.
        if event.origin == Origin::Runtime
            || event.kind.explicit()
            || event.goal_id.is_some_and(|id| id <= 0)
        {
            return;
        }
        self.diagnostics.last_event = Some(event.kind);
        self.cancel();
        if let Some(index) = self.pending.iter().position(|old| {
            old.kind == event.kind && old.goal_id == event.goal_id && old.step_id == event.step_id
        }) {
            self.pending.remove(index);
        }
        if self.pending.len() == MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(event);
    }
    fn ready(&mut self, now: i64, allowed: bool) -> Option<Event> {
        self.pending
            .retain(|event| now >= event.at && now - event.at <= EVENT_TTL);
        if !allowed {
            self.cancel();
            self.pending.clear();
            self.suppress("privacy_or_quiet");
            return None;
        }
        if self.flight.is_some() {
            self.suppress("in_flight");
            return None;
        }
        if now.saturating_sub(self.saved.last_attempt) < COOLDOWN {
            self.suppress("cooldown");
            return None;
        }
        if now - self.pending.back()?.at < DEBOUNCE {
            self.suppress("debounce");
            return None;
        }
        self.diagnostics.suppression_reason = None;
        let event = self.pending.pop_back()?;
        // One consolidated reasoning pass covers the current state, not a replay of old ticks.
        self.pending.clear();
        Some(event)
    }
    fn suppress(&mut self, reason: &'static str) {
        self.diagnostics.suppression_reason = Some(reason);
        self.diagnostics.suppressed = self.diagnostics.suppressed.saturating_add(1);
    }
}
/// Explicit input preempts background work; it is handled immediately by the shared chat pipeline.
pub(crate) fn diagnostics_view(inner: &Inner) -> Value {
    json!({"status":if inner.agent.flight.is_some(){"reasoning"}else if !background_allowed(inner){"suppressed"}else{"idle"},"pending_events":inner.agent.pending.len(),"diagnostics":inner.agent.diagnostics})
}
pub fn begin_explicit(inner: &mut Inner, kind: EventKind) {
    debug_assert!(kind.explicit());
    inner.agent.cancel();
    inner.agent.pending.clear();
    inner.agent.diagnostics.last_input = Some(kind);
    crate::engine_observability::record(
        inner,
        "input",
        if kind == EventKind::VoiceMessage {
            "voice_message"
        } else {
            "user_message"
        },
        "received",
        None,
        None,
    );
}
/// Shared post-transcript action entry. Audio is accepted only by the local STT IPC.
pub fn explicit_event_kind(request: &Value) -> Result<EventKind, String> {
    match request.get("event_type").and_then(Value::as_str) {
        None | Some("user_message") => Ok(EventKind::UserMessage),
        Some("voice_message") => Ok(EventKind::VoiceMessage),
        _ => Err("An explicit text or voice message is required".into()),
    }
}
pub async fn handle_dialogue(
    request: &Value,
    conversation: i64,
    state: &AppState,
    revision: u64,
    share: bool,
    service: &Service<'_>,
) -> Result<crate::agent_dialogue::Outcome, String> {
    explicit_event_kind(request)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(90),
        crate::agent_dialogue::run(request, conversation, state, revision, share, service),
    )
    .await
    .map_err(|_| {
        "The agent request timed out. Check the verified app state before retrying.".to_string()
    })?
}
pub async fn handle_explicit_actions(
    request: &Value,
    conversation: i64,
    state: &AppState,
    revision: u64,
    share: bool,
    service: &Service<'_>,
) -> Result<Option<(String, u64, bool)>, String> {
    explicit_event_kind(request)?;
    crate::ai_chat::structured_chat_actions_with_service(
        request,
        conversation,
        state,
        revision,
        share,
        service,
    )
    .await
}
fn service_allowed(inner: &Inner) -> bool {
    let minute = Local::now().hour() * 60 + Local::now().minute();
    inner
        .storage
        .ai_preferences()
        .is_ok_and(|p| p.enabled && p.share_goal_context)
        && inner
            .storage
            .user_settings()
            .is_ok_and(|s| s.onboarding.completed)
        && (inner.status.anthropic_configured || inner.status.mock_ai)
        && !crate::companion::quiet_mode(inner)
        && !inner.companion.view.preferences.paused
        && !inner.companion.view.preferences.quiet(minute)
}
fn background_allowed(inner: &Inner) -> bool {
    service_allowed(inner)
        && (inner.status.ai_enabled || inner.companion.view.preferences.screen_task_detection)
        && inner.status.tracking
        && inner.status.tracking_error.is_none()
        && crate::tracking::notifications_allowed(inner)
        && inner.buddy.foreground.as_ref().is_some_and(|a| {
            a.idle_seconds < inner.tracking_settings.idle_seconds
                && !a.media_playing
                && !crate::attention::meeting(a)
                && !crate::attention::excluded(&inner.buddy.view.preferences, &a.process_name)
                && !crate::companion::sensitive_window(a)
        })
}
pub struct Job {
    event: Event,
    snapshot: skills::Snapshot,
    privacy: u64,
    generation: u64,
    cancel: watch::Receiver<bool>,
    source: Value,
}
fn prepare(inner: &mut Inner, now: i64) -> Result<Option<Job>, String> {
    let allowed = background_allowed(inner);
    let Some(event) = inner.agent.ready(now, allowed) else {
        return Ok(None);
    };
    if matches!(
        event.kind,
        EventKind::PossibleStuck | EventKind::PossibleDrift
    ) && !inner.storage.adaptive_offer_allowed(
        event.goal_id,
        event.kind == EventKind::PossibleStuck,
        now,
        inner.companion.memory.last_prompt,
    ) {
        inner.agent.suppress("adaptive_policy");
        return Ok(None);
    }
    let conversation = inner.storage.current_conversation()?;
    let snapshot = skills::snapshot(&inner.storage, conversation)?;
    if event.goal_id.is_some_and(|id| {
        !snapshot.data["goals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["goal_id"] == id)
    }) {
        return Ok(None);
    }
    let step = event
        .goal_id
        .map(|id| inner.storage.goal_plan(id).map(|p| p.current_step))
        .transpose()?
        .flatten();
    let app = inner
        .buddy
        .foreground
        .as_ref()
        .and_then(|a| crate::memory::safe_text(&a.process_name, 120).ok());
    let memory = inner
        .storage
        .retrieve_contextual_memory(
            &crate::memory::retriever::RetrievalInput {
                exact_scope: event.goal_id.map(crate::memory::Scope::goal),
                active_goal_id: event.goal_id,
                active_step_id: step,
                current_app: app.clone(),
                ..Default::default()
            },
            &Default::default(),
            true,
        )?
        .context;
    let goal = snapshot.data["goals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| Some(g["goal_id"].as_i64().unwrap()) == event.goal_id)
        .cloned();
    let work_state = crate::work_state::compute(inner)?;
    // Checkpoint text remains exclusively in the scoped, budgeted memory envelope.
    let understanding = json!({"personal_work_state":{"active_goal_id":work_state.active_goal_id,"active_step_id":work_state.active_step_id,"state":work_state.state,"confidence":work_state.confidence,"relevant_evidence":work_state.relevant_evidence},"activity_state":inner.activity_state.state,"possible_stuck":inner.agent.observation.stuck,"possible_drift":inner.agent.observation.drift});
    let continuity = match event.kind {
        EventKind::DayStarted => inner
            .storage
            .continuity_report(
                crate::continuity::View::Priorities,
                None,
                Local::now().date_naive(),
            )
            .ok()
            .map(|r| r.render()),
        EventKind::DayEnded => inner
            .storage
            .continuity_report(
                crate::continuity::View::Yesterday,
                None,
                Local::now().date_naive(),
            )
            .ok()
            .map(|r| r.render()),
        _ => None,
    };
    let policy = inner.storage.adaptive_policy(event.goal_id, Utc::now())?;
    let source = json!({"daily_continuity":continuity,"adaptive_policy":policy,"observation":understanding,"event":event,"state_version":snapshot.version,"goal":goal,"current_app":app,"personal_memory":memory,"registry":skills::planning_catalog()});
    let (cancel, receiver) = watch::channel(false);
    crate::engine_observability::record(
        inner,
        "reason",
        serde_json::to_value(event.kind).unwrap().as_str().unwrap(),
        "scheduled",
        None,
        event.goal_id,
    );
    inner.agent.saved.last_attempt = now;
    inner
        .storage
        .write_setting("agent_runtime_state", &inner.agent.saved)?;
    let generation = inner.agent.generation;
    inner.agent.flight = Some(Flight {
        generation,
        privacy: inner.privacy_revision,
        data: snapshot.data.clone(),
        conversation,
        cancel,
    });
    if !inner.status.mock_ai {
        inner.agent.diagnostics.ai_calls = inner.agent.diagnostics.ai_calls.saturating_add(1);
    }
    Ok(Some(Job {
        event,
        snapshot,
        privacy: inner.privacy_revision,
        generation,
        cancel: receiver,
        source,
    }))
}
fn valid(inner: &Inner, job: &Job) -> bool {
    inner
        .agent
        .flight
        .as_ref()
        .is_some_and(|f| f.generation == job.generation)
        && inner.privacy_revision == job.privacy
        && background_allowed(inner)
        && inner.storage.current_conversation().ok() == Some(job.snapshot.conversation)
        && skills::snapshot(&inner.storage, job.snapshot.conversation)
            .is_ok_and(|s| s.data == job.snapshot.data)
}
async fn reason(
    state: &AppState,
    job: &mut Job,
    service: &Service<'_>,
) -> Result<Reasoning, String> {
    if *job.cancel.borrow() {
        return Err("Agent request cancelled".into());
    }
    tokio::select! {
        result=crate::engine_observability::request_validated(state,service,reasoning::BACKGROUND_PROMPT,job.source.clone(),"buddy_agent_reasoning",reasoning::background_schema(),reasoning::parse) => result,
        _=job.cancel.changed() => Err("Agent request cancelled".into()),
    }
}
fn apply(inner: &mut Inner, job: &Job, result: &Reasoning) -> Result<Option<String>, String> {
    if !valid(inner, job) {
        return Err("Agent response is stale".into());
    }
    result.validate_for(&job.event, &job.snapshot)?;
    for action in &result.proposed_actions {
        skills::validate_offer(&inner.storage, &job.snapshot, action, job.event.goal_id)?;
    }
    let mut message = match result.decision {
        Decision::StaySilent | Decision::UpdateMemoryOnly => None,
        Decision::OfferHelp | Decision::OfferSkill => Some(
            if let Some(next) =
                job.source["personal_memory"]["work_context"]["next_action"].as_str()
            {
                format!("Ready to continue? Your saved next action: {next}")
            } else {
                "Would you like help choosing the next action for this goal?".into()
            },
        ),
        Decision::RequestClarification => result.clarification.clone(),
        Decision::ExecuteExplicitUserAction => {
            return Err("Background events cannot authorize skill execution".into())
        }
    };
    if message.is_some() && matches!(job.event.kind, EventKind::DayStarted | EventKind::DayEnded) {
        // Day summaries bypass model prose just like explicit continuity read skills.
        message = job.source["daily_continuity"].as_str().map(str::to_owned);
    }
    // Verified domain events, not model prose, own episodic memory identities and content.
    reasoning::remember_verified(&inner.storage, &job.event, &result.memory_updates)?;
    crate::engine_observability::record(
        inner,
        "decision",
        serde_json::to_value(result.decision)
            .unwrap()
            .as_str()
            .unwrap(),
        "verified",
        None,
        job.event.goal_id,
    );
    inner.agent.diagnostics.last_decision = Some(result.decision);
    inner.agent.diagnostics.verified = inner.agent.diagnostics.verified.saturating_add(1);
    // Absorb any state changed by our own memory write. No memory-created event is emitted.
    inner.agent.flight = None;
    Ok(message)
}
struct VerifiedOffer {
    event: Event,
    text: String,
    data: Value,
    privacy: u64,
    conversation: i64,
}
impl VerifiedOffer {
    fn current(&self, inner: &Inner) -> bool {
        background_allowed(inner)
            && inner.privacy_revision == self.privacy
            && inner.storage.current_conversation().ok() == Some(self.conversation)
            && inner.storage.goal().ok().flatten().map(|g| g.id) == self.event.goal_id
            && skills::snapshot(&inner.storage, self.conversation)
                .is_ok_and(|s| s.data == self.data)
    }
}
async fn process_pending_with_service(
    state: &AppState,
    service: &Service<'_>,
    app: Option<&AppHandle>,
) -> Result<Option<VerifiedOffer>, String> {
    let Some(mut job) = ({
        let mut inner = state.inner.lock().map_err(|_| "Agent state unavailable")?;
        let job = prepare(&mut inner, Utc::now().timestamp())?;
        if let Some(app) = app {
            crate::buddy::sync(app, &mut inner)?;
        }
        job
    }) else {
        return Ok(None);
    };
    let mock = state
        .inner
        .lock()
        .map_err(|_| "Agent state unavailable")?
        .status
        .mock_ai;
    let outcome = if mock {
        Ok(Reasoning::silent(job.snapshot.version.clone()))
    } else {
        reason(state, &mut job, service).await
    };
    let mut inner = state.inner.lock().map_err(|_| "Agent state unavailable")?;
    let applied = outcome.and_then(|result| apply(&mut inner, &job, &result));
    // Keep only a fixed diagnostic code, never provider text or private context.
    inner.agent.diagnostics.last_failure = applied
        .as_ref()
        .err()
        .map(|error| crate::engine_observability::error_category(error));
    if let Some(code) = inner.agent.diagnostics.last_failure {
        inner.agent.suppress(code);
        crate::engine_observability::record(
            &mut inner,
            "decision",
            "background_reasoning",
            code,
            None,
            job.event.goal_id,
        );
    }
    if inner
        .agent
        .flight
        .as_ref()
        .is_some_and(|f| f.generation == job.generation)
    {
        inner.agent.flight = None;
    }
    if let Some(app) = app {
        crate::buddy::sync(app, &mut inner)?;
    }
    let Some(text) = applied? else {
        return Ok(None);
    };
    Ok(Some(VerifiedOffer {
        event: job.event,
        text,
        data: skills::snapshot(&inner.storage, job.snapshot.conversation)?.data,
        privacy: inner.privacy_revision,
        conversation: job.snapshot.conversation,
    }))
}
#[cfg(test)]
async fn run_pending_with_service(
    state: &AppState,
    service: &Service<'_>,
) -> Result<Option<VerifiedOffer>, String> {
    let Ok(_guard) = state.companion_request.try_lock() else {
        return Ok(None);
    };
    process_pending_with_service(state, service, None).await
}
async fn run_pending(app: &AppHandle) {
    let state = app.state::<AppState>();
    let ready = state.inner.lock().is_ok_and(|i| {
        background_allowed(&i)
            && i.agent.flight.is_none()
            && Utc::now()
                .timestamp()
                .saturating_sub(i.agent.saved.last_attempt)
                >= COOLDOWN
            && i.agent
                .pending
                .iter()
                .any(|e| Utc::now().timestamp().saturating_sub(e.at) >= DEBOUNCE)
    });
    if !ready {
        return;
    }
    let Ok(_guard) = state.companion_request.try_lock() else {
        return;
    };
    let service = if state.inner.lock().is_ok_and(|i| i.status.mock_ai) {
        Service {
            client: &state.client,
            endpoint: String::new(),
            key: String::new(),
            model: String::new(),
        }
    } else {
        match Service::configured(&state.client) {
            Ok(service) => service,
            Err(error) => {
                if let Ok(mut inner) = state.inner.lock() {
                    let code = crate::engine_observability::error_category(&error);
                    inner.agent.diagnostics.last_failure = Some(code);
                    inner.agent.suppress(code);
                }
                return;
            }
        }
    };
    match process_pending_with_service(&state, &service, Some(app)).await {
        Ok(Some(offer)) => {
            if let Ok(mut inner) = state.inner.lock() {
                // Re-check presentation gates too; a response is never a permission to interrupt.
                if offer.current(&inner) {
                    let _ = crate::companion::offer_agent(
                        app,
                        &mut inner,
                        offer.event.goal_id,
                        &offer.text,
                        offer.event.kind == EventKind::PossibleStuck,
                        offer.event.step_id.as_deref(),
                    );
                }
            }
        }
        Err(error) => {
            if let Ok(mut inner) = state.inner.lock() {
                let code = crate::engine_observability::error_category(&error);
                // Also cover preparation/storage failures before inference starts.
                if inner.agent.diagnostics.suppression_reason != Some(code) {
                    inner.agent.diagnostics.last_failure = Some(code);
                    inner.agent.suppress(code);
                }
            }
        }
        _ => {}
    }
}
/// Sole periodic entry point. Collection stays local; network jobs are claimed and run off-thread.
pub async fn tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Ok(mut inner) = state.inner.lock() {
        if let Err(error) = crate::automatic_goals::sync_focus(&mut inner) {
            inner.last_error = Some(error);
        }
        if let Err(error) = inner.storage.core_tick_at(Utc::now()) {
            inner.last_error = Some(error);
        }
    }
    if let Err(error) = crate::commands::collect(&state) {
        if let Ok(mut inner) = state.inner.lock() {
            if error != crate::collector::CONTEXT_CHANGED {
                inner.last_error = Some(error);
            }
            inner.buddy.foreground = None;
            inner.usage = Default::default();
            inner.activity_state.stop(false);
        }
    }
    let (allowed, significant) = if let Ok(mut inner) = state.inner.lock() {
        if let Err(error) = crate::privacy::cleanup_due(&mut inner) {
            inner.last_error = Some(error);
        }
        if let Err(error) = crate::buddy::sync(app, &mut inner) {
            inner.last_error = Some(error);
        }
        let before = inner.agent.generation;
        let observed = observe::observe(&mut inner, Utc::now().timestamp());
        let allowed = observed.is_ok() && background_allowed(&inner);
        if let Err(error) = observed {
            inner.last_error = Some(error);
        }
        let stale = inner.agent.flight.as_ref().is_some_and(|f| {
            f.privacy != inner.privacy_revision
                || inner.storage.current_conversation().ok() != Some(f.conversation)
                || skills::snapshot(
                    &inner.storage,
                    inner.storage.current_conversation().unwrap_or_default(),
                )
                .map_or(true, |s| s.data != f.data)
        });
        if !allowed || stale {
            inner.agent.cancel();
        }
        if !allowed {
            inner.agent.pending.clear();
        }
        (allowed, inner.agent.generation != before)
    } else {
        (false, false)
    };
    {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            if let Err(error) = crate::activity_review::run_due(&handle, &state).await {
                if let Ok(mut inner) = state.inner.lock() {
                    inner.last_error = Some(error);
                }
            }
        });
    }
    // Work outside the goal list is offered after a few unmatched minutes, alongside the
    // unified observation pass; both share the provider lock and the hourly budget.
    if crate::unplanned_work::due(&state) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::unplanned_work::identify(&handle, &handle.state::<AppState>()).await;
        });
    }
    // Existing specialist adapters retain their own consent, stable-context windows and cooldowns.
    if state.inner.lock().is_ok_and(|i| service_allowed(&i)) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::automatic_goals::run_pending(&handle).await;
        });
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::goal_understanding::run_due(&handle).await;
        });
    }
    // Goal attribution is background accounting, not a prompt: it is the primary way
    // measured time reaches a goal and continues through quiet hours.
    if crate::goal_matching::due(&state) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::goal_matching::identify(&handle, &handle.state::<AppState>()).await;
        });
    }
    if allowed {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            run_pending(&handle).await;
        });
        if significant && crate::commands::should_analyze(&state) {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::commands::automatic_analysis(&handle, &handle.state::<AppState>()).await;
            });
        }
        // The legacy proactive cadence can offer research, never authorize a web search call.
        if let Ok(mut inner) = state.inner.lock() {
            if crate::buddy::due(&inner) {
                let goal = inner.storage.goal().ok().flatten().map(|g| g.id);
                let _ = crate::companion::offer_agent(
                    app,
                    &mut inner,
                    goal,
                    "Search for information for this step?",
                    true,
                    None,
                );
                if inner
                    .companion
                    .view
                    .intervention
                    .as_ref()
                    .is_some_and(|p| p.kind == crate::companion::engine::Kind::Research)
                {
                    inner.buddy.last_search = Some(std::time::Instant::now());
                    inner.buddy.last_attempt_at = Some(Utc::now().timestamp());
                    let _ = inner
                        .storage
                        .write_setting("buddy_last_attempt", &inner.buddy.last_attempt_at);
                }
            }
        }
    }
    // Observation is independent of media playback and Quiet-mode presentation policy.
    if state
        .inner
        .lock()
        .is_ok_and(|i| crate::companion::detection_due(&i))
    {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            if let Err(error) = crate::companion::detect(&handle, &state).await {
                if let Ok(mut inner) = state.inner.lock() {
                    inner.companion.semantic.status = "failed";
                    inner.companion.semantic.reason = "provider_or_validation_failure";
                    inner.companion.progress_error = Some(error);
                    let _ = crate::buddy::sync(&handle, &mut inner);
                }
            }
        });
    }
}
