//! Local, content-free engine diagnostics and an explicitly requested synthetic inference probe.
use crate::{
    commands::{AppState, Inner},
    goal_analysis::{self, Service},
    http, skills,
};
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{State, WebviewWindow};
mod action_check;
pub(crate) async fn headless_action_check() -> Result<Vec<action_check::CheckResult>, String> {
    let state = AppState::new(crate::storage::Storage::open(std::path::Path::new(
        ":memory:",
    ))?)?;
    if state.inner.lock().unwrap().status.mock_ai {
        return Err("Live check requires real provider responses".into());
    }
    let service = Service::configured(&state.client)?;
    action_check_with_service(&service, &state).await
}
pub(crate) async fn action_check_with_service(
    service: &Service<'_>,
    state: &AppState,
) -> Result<Vec<action_check::CheckResult>, String> {
    tokio::time::timeout(
        Duration::from_secs(360),
        action_check::run(service, state, 0),
    )
    .await
    .map_err(|_| "Agent check timed out")?
}

#[derive(Clone, Debug, Serialize)]
pub struct Inference {
    operation: String,
    status: &'static str,
    latency_ms: u64,
    provider_reported_model: Option<String>,
    finish_reason: Option<String>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    validation_error: Option<&'static str>,
}
pub(crate) fn error_category(error: &str) -> &'static str {
    if error == "Invalid agent reasoning JSON" {
        "reasoning_json_invalid"
    } else if error == "Invalid agent reasoning fields" {
        "reasoning_fields_invalid"
    } else if error == "Invalid agent memory update" {
        "reasoning_memory_invalid"
    } else if error == "Unknown agent skill" {
        "reasoning_skill_unknown"
    } else if error.contains("cancelled") {
        "cancelled"
    } else if error.contains("HTTP 401") || error.contains("HTTP 403") {
        "provider_authentication"
    } else if error.contains("HTTP 402") {
        "provider_billing"
    } else if error.contains("HTTP 429") {
        "provider_rate_limit"
    } else if error.contains("HTTP 5") {
        "provider_unavailable"
    } else if error.contains("timed out") {
        "provider_timeout_or_network"
    } else if error.starts_with("Configure ") || error.contains("Credential access") {
        "credentials_unavailable"
    } else if error == "Service returned invalid JSON"
        || error == "The AI returned an invalid response"
    {
        "provider_response_invalid"
    } else if error.contains("does not match") {
        "operation_mismatch"
    } else if error.contains("read-only") {
        "read_only_request"
    } else if error.contains("instruction") {
        "instruction_required"
    } else if error.contains("stale") || error.contains("changed") {
        "stale_context"
    } else if error.contains("incomplete") {
        "incomplete_output"
    } else if error.contains("goal") || error.contains("step") {
        "target_validation"
    } else {
        "schema_or_provider_error"
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub latency_ms: Option<u64>,
    pub provider_reported_model: Option<String>,
    pub error: Option<&'static str>,
    pub checked_at: Option<String>,
}
impl Default for Health {
    fn default() -> Self {
        Self {
            status: "not_checked",
            latency_ms: None,
            provider_reported_model: None,
            error: None,
            checked_at: None,
        }
    }
}
#[derive(Default)]
pub struct Telemetry {
    action_check_started: Option<Instant>,
    events: VecDeque<EngineEvent>,
    connections: [Option<ConnectionHealth>; 2],
    checking_connections: [bool; 2],
    sequence: u64,
    last: Option<Inference>,
    last_skill: Option<String>,
    privacy: u64,
    health: Health,
    health_privacy: u64,
    health_started: Option<Instant>,
    health_running: bool,
}
#[tauri::command(async)]
pub async fn test_agent_actions(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Vec<action_check::CheckResult>, String> {
    main_window(window.label())?;
    let _guard = state
        .analysis
        .try_lock()
        .map_err(|_| "An AI check is already running")?;
    let revision = {
        let mut inner = state.inner.lock().map_err(|_| "Engine state unavailable")?;
        if inner.status.mock_ai || !inner.storage.ai_preferences()?.enabled {
            return Err("Enable real AI assistance before testing agent actions".into());
        }
        if inner
            .engine
            .action_check_started
            .is_some_and(|last| last.elapsed() < Duration::from_secs(60))
        {
            return Err("Wait 60 seconds between agent checks".into());
        }
        inner.engine.action_check_started = Some(Instant::now());
        record(
            &mut inner,
            "check",
            "synthetic_agent_actions",
            "started",
            None,
            None,
        );
        inner.privacy_revision
    };
    let service = Service::configured(&state.client)?;
    let result = tokio::time::timeout(
        Duration::from_secs(360),
        action_check::run(&service, &state, revision),
    )
    .await;
    let mut inner = state.inner.lock().map_err(|_| "Engine state unavailable")?;
    record(
        &mut inner,
        "check",
        "synthetic_agent_actions",
        if matches!(&result,Ok(Ok(results)) if results.len()==9 && results.iter().all(|r|r.status=="passed"))
        {
            "passed"
        } else {
            "failed"
        },
        None,
        None,
    );
    result.map_err(|_| "Agent check timed out; no user data was changed")?
}
#[derive(Clone, Serialize)]
pub struct EngineEvent {
    at: String,
    stage: &'static str,
    operation: String,
    status: &'static str,
    latency_ms: Option<u64>,
    goal_id: Option<i64>,
}
/// Bounded, session-only journal. Never store prompts, model reasoning or collected text.
pub(crate) fn record(
    inner: &mut Inner,
    stage: &'static str,
    operation: &str,
    status: &'static str,
    latency_ms: Option<u64>,
    goal_id: Option<i64>,
) {
    let safe = operation.len() <= 100
        && operation
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_:-".contains(&c));
    if !safe {
        return;
    }
    if inner.engine.events.len() == 200 {
        inner.engine.events.pop_front();
    }
    inner.engine.events.push_back(EngineEvent {
        at: chrono::Utc::now().to_rfc3339(),
        stage,
        operation: operation.into(),
        status,
        latency_ms,
        goal_id,
    });
}
impl Telemetry {
    pub(crate) fn privacy_revision_matches(&self, revision: u64) -> bool {
        self.privacy == revision
    }
    pub(crate) fn advance_domain_revision(&mut self, revision: u64) {
        self.privacy = revision;
    }
}
#[derive(Clone)]
struct ConnectionHealth {
    connected: bool,
    model_available: Option<bool>,
    privacy: u64,
    checked: Instant,
    checked_at: String,
}
fn provider_index(provider: &str) -> Result<usize, String> {
    match provider {
        "anthropic" => Ok(0),
        "tavily" => Ok(1),
        _ => Err("Unknown provider".into()),
    }
}
pub(crate) fn invalidate_connections(inner: &mut Inner) {
    inner.engine.connections = [None, None];
    inner.engine.checking_connections = [false, false];
    inner.engine.health = Default::default();
}
pub(crate) fn begin_connection_check(inner: &mut Inner, provider: &str) -> Result<(), String> {
    let index = provider_index(provider)?;
    if inner.engine.checking_connections[index] {
        return Err("A connection check is already running".into());
    }
    if inner.engine.connections[index].as_ref().is_some_and(|c| {
        c.privacy == inner.last_consent_change && c.checked.elapsed() < Duration::from_secs(10)
    }) {
        return Err("Wait a few seconds before checking this connection again".into());
    }
    inner.engine.checking_connections[index] = true;
    Ok(())
}
pub(crate) fn finish_connection_check(
    inner: &mut Inner,
    provider: &str,
    revision: u64,
    connected: bool,
    model_available: Option<bool>,
) {
    let Ok(index) = provider_index(provider) else {
        return;
    };
    if inner.last_consent_change != revision {
        return;
    }
    inner.engine.checking_connections[index] = false;
    // Startup metadata can predate Keychain authorization or a completed app update.
    // A successful current probe proves that a usable runtime credential exists.
    if connected {
        if provider == "anthropic" {
            inner.status.anthropic_configured = true;
        } else {
            inner.status.tavily_configured = true;
        }
    }
    inner.engine.connections[index] = Some(ConnectionHealth {
        connected,
        model_available,
        privacy: revision,
        checked: Instant::now(),
        checked_at: chrono::Utc::now().to_rfc3339(),
    });
}
fn service_status_view(inner: &Inner) -> Value {
    let configured_model = http::secret("ANTHROPIC_MODEL_ID")
        .ok()
        .as_deref()
        .and_then(model_label);
    let services: Vec<Value> = [
        ("anthropic", 0, inner.status.anthropic_configured),
        (
            "model",
            0,
            inner.status.anthropic_configured && configured_model.is_some(),
        ),
        ("tavily", 1, inner.status.tavily_configured),
    ]
    .into_iter()
    .map(|(id, index, configured)| {
        let check = inner.engine.connections[index]
            .as_ref()
            .filter(|c| c.privacy == inner.last_consent_change);
        let status = if inner.status.mock_ai {
            "mock"
        } else if !configured {
            "not_configured"
        } else if inner.engine.checking_connections[index] {
            "checking"
        } else {
            match check {
                None => "not_checked",
                Some(c) if c.checked.elapsed() > Duration::from_secs(15 * 60) => "expired",
                Some(c) if !c.connected || (id == "model" && c.model_available != Some(true)) => {
                    "unavailable"
                }
                Some(_) => "verified",
            }
        };
        json!({"id":id,"status":status,"checked_at":check.map(|c|c.checked_at.as_str())})
    })
    .collect();
    json!({
        "connection_mode": crate::shared_ai::mode(),
        "configured_model": configured_model,
        "configured_response_model": http::secret("ANTHROPIC_MODEL_ID")
            .ok().map(|m| http::response_model(&m)).as_deref().and_then(model_label),
        "configured_vision_model": model_label(&crate::vision::configured_model(inner)),
        "configured_image_description_model": model_label(&crate::vision::image_model()),
        "services": services,
    })
}
#[tauri::command(async)]
pub fn get_ai_service_status(
    window: WebviewWindow,
    state: State<AppState>,
) -> Result<Value, String> {
    main_window(window.label())?;
    let inner = state.inner.lock().map_err(|_| "Engine state unavailable")?;
    Ok(service_status_view(&inner))
}
fn model_label(value: &str) -> Option<String> {
    (value.len() <= 160
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_:.-".contains(&b)))
    .then(|| crate::memory::safe_text(value, 160).ok())
    .flatten()
}
fn label(value: &str) -> String {
    crate::memory::safe_text(value, 500).unwrap_or_else(|_| "[Private label]".into())
}
fn elapsed(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}
struct InferenceGuard<'a> {
    state: &'a AppState,
    sequence: u64,
    privacy: u64,
    operation: String,
    start: Instant,
    finished: bool,
}
impl<'a> InferenceGuard<'a> {
    fn start(state: &'a AppState, operation: &str) -> Result<Self, String> {
        let mut inner = state.inner.lock().map_err(|_| "Engine state unavailable")?;
        inner.engine.sequence = inner.engine.sequence.wrapping_add(1);
        record(&mut inner, "inference", operation, "started", None, None);
        Ok(Self {
            state,
            sequence: inner.engine.sequence,
            privacy: inner.privacy_revision,
            operation: operation.into(),
            start: Instant::now(),
            finished: false,
        })
    }
    fn finish(&mut self, status: &'static str, model: Option<String>) {
        if let Ok(mut inner) = self.state.inner.lock() {
            if inner.privacy_revision == self.privacy {
                record(
                    &mut inner,
                    "inference",
                    &self.operation,
                    status,
                    Some(elapsed(self.start)),
                    None,
                );
            }
            if inner.engine.sequence == self.sequence && inner.privacy_revision == self.privacy {
                inner.engine.privacy = self.privacy;
                inner.engine.last = Some(Inference {
                    operation: self.operation.clone(),
                    status,
                    latency_ms: elapsed(self.start),
                    provider_reported_model: model,
                    finish_reason: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    validation_error: None,
                });
            }
        }
        self.finished = true;
    }
}
impl Drop for InferenceGuard<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.finish("cancelled", None);
        }
    }
}
/// Instrument existing specialist adapters without changing their validation/retry rules.
pub(crate) async fn observe_specialist<T>(
    state: &AppState,
    name: &str,
    request: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let mut guard = InferenceGuard::start(state, name)?;
    let result = request.await;
    guard.finish(if result.is_ok() { "success" } else { "error" }, None);
    if let Err(error) = &result {
        if let Ok(mut inner) = state.inner.lock() {
            if inner.privacy_revision == guard.privacy {
                record(
                    &mut inner,
                    "validation",
                    error_category(error),
                    "rejected",
                    None,
                    None,
                );
            }
        }
    }
    result
}
/// Existing retry/validation logic stays authoritative. Dropped requests are recorded as cancelled.
pub async fn request_prose<T>(
    state: &AppState,
    service: &Service<'_>,
    system: &str,
    context: Value,
    parse: impl Fn(&Value) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = InferenceGuard::start(state, "buddy_dialogue_response")?;
    let mut last_error = "Invalid natural response".to_string();
    for attempt in 0..2 {
        let instructions = if attempt == 0 {
            system.to_string()
        } else {
            format!("{system} The previous response failed validation. Return complete plain text within 6000 characters, using only supplied source URLs.")
        };
        let value = match service.request_prose(&instructions, context.clone()).await {
            Ok(value) => value,
            Err(error) => {
                guard.finish("error", None);
                return Err(error);
            }
        };
        let result = parse(&value);
        if result.is_ok() || attempt == 1 {
            guard.finish(
                if result.is_ok() { "success" } else { "error" },
                value["model"].as_str().and_then(model_label),
            );
            if let Ok(mut inner) = state.inner.lock() {
                if inner.privacy_revision == guard.privacy
                    && inner.engine.sequence == guard.sequence
                {
                    if let Some(last) = &mut inner.engine.last {
                        last.finish_reason = value["choices"][0]["finish_reason"]
                            .as_str()
                            .filter(|s| ["stop", "length", "content_filter"].contains(s))
                            .map(str::to_string);
                        last.prompt_tokens = value["usage"]["prompt_tokens"].as_u64();
                        last.completion_tokens = value["usage"]["completion_tokens"].as_u64();
                        last.validation_error = result.as_ref().err().map(|e| error_category(e));
                    }
                }
            }
            return result;
        }
        last_error = result.err().unwrap();
    }
    Err(last_error)
}

pub async fn request_validated<T>(
    state: &AppState,
    service: &Service<'_>,
    system: &str,
    context: Value,
    name: &str,
    schema: Value,
    parse: impl Fn(&Value) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = InferenceGuard::start(state, name)?;
    let reported = Mutex::new((None, None, None, None));
    let result = service
        .request_validated(system, context, name, schema, |value| {
            if let Ok(mut metadata) = reported.lock() {
                *metadata = (
                    value["model"].as_str().and_then(model_label),
                    value
                        .pointer("/choices/0/finish_reason")
                        .and_then(Value::as_str)
                        .filter(|s| ["stop", "length", "tool_calls", "content_filter"].contains(s))
                        .map(str::to_string),
                    value["usage"]["prompt_tokens"].as_u64(),
                    value["usage"]["completion_tokens"].as_u64(),
                );
            }
            parse(value)
        })
        .await;
    let metadata = reported.into_inner().unwrap_or_default();
    guard.finish(if result.is_ok() { "success" } else { "error" }, metadata.0);
    if let Ok(mut inner) = state.inner.lock() {
        if inner.privacy_revision == guard.privacy && inner.engine.sequence == guard.sequence {
            if let Some(last) = &mut inner.engine.last {
                last.finish_reason = metadata.1;
                last.prompt_tokens = metadata.2;
                last.completion_tokens = metadata.3;
                last.validation_error = result.as_ref().err().map(|e| error_category(e));
            }
            if let Err(error) = &result {
                record(
                    &mut inner,
                    "validation",
                    error_category(error),
                    "rejected",
                    None,
                    None,
                );
            }
        }
    }
    result
}
pub(crate) fn record_execution(inner: &mut Inner, execution: &skills::Execution) {
    for result in &execution.results {
        record(
            inner,
            "skill",
            &result.skill,
            execution.status,
            None,
            result.goal_id,
        );
    }
    if execution.status == "executed" {
        inner.engine.last_skill = execution.results.last().map(|r| r.skill.clone());
    }
}
fn skill_groups() -> Value {
    let groups = [
        (
            "Goals",
            vec![
                "list_goals",
                "inspect_work_queue",
                "inspect_goal",
                "create_goal",
                "rename_goal",
                "delete_goal",
                "restore_goal",
                "add_goal_to_today",
                "remove_goal_from_today",
                "set_focus_goal",
                "create_planned_goal",
                "update_goal",
                "find_goals",
                "schedule_goal",
                "undo_last_goal_edit",
            ],
        ),
        (
            "Steps",
            vec![
                "create_step",
                "rename_step",
                "delete_step",
                "reorder_steps",
                "set_current_step",
                "add_research_step",
            ],
        ),
        ("Memory", vec!["inspect_conversation"]),
        (
            "Work Context",
            vec![
                "park_context",
                "resume_context",
                "inspect_work_context",
                "update_work_context",
            ],
        ),
        (
            "Research",
            vec![
                "search_web",
                "attach_research_to_context",
                "record_research_feedback",
                "inspect_research",
            ],
        ),
        (
            "Completion Verification",
            vec![
                "complete_step",
                "reopen_step",
                "complete_goal",
                "reopen_goal",
                "restart_goal",
            ],
        ),
        ("Daily Review", vec!["inspect_today", "inspect_continuity"]),
    ];
    let registry = skills::registry();
    json!(groups.into_iter().map(|(group,names)|json!({"group":group,"skills":registry.iter().filter(|s|names.contains(&s.name)).map(|s|json!({"name":s.name,"permission":s.safety})).collect::<Vec<_>>(),"note":if group=="Memory" {"Validated local Memory APIs and reviewed preference proposals; no unrestricted memory-write skill."}else{""}})).collect::<Vec<_>>())
}
pub(crate) fn view(inner: &Inner) -> Result<Value, String> {
    let ai = inner.storage.ai_preferences()?;
    let memory = inner.storage.memory_controls()?;
    let sharing = ai.enabled && ai.share_goal_context;
    let goal = if sharing { inner.storage.goal()? } else { None };
    let plan = goal
        .as_ref()
        .map(|g| inner.storage.goal_plan(g.id))
        .transpose()?;
    let step = plan.as_ref().and_then(|p| {
        p.current_step
            .as_ref()
            .and_then(|id| p.steps.iter().find(|s| &s.id == id))
    });
    let refs = inner
        .storage
        .action_references(inner.storage.current_conversation()?)?;
    let last_skill = if sharing {
        inner
            .engine
            .last_skill
            .as_deref()
            .or_else(|| refs.last_action.as_ref().and_then(|a| a["skill"].as_str()))
            .filter(|n| skills::registry().iter().any(|s| s.name == *n))
    } else {
        None
    };
    let metadata: Option<String> = if memory.enabled && sharing {
        inner
            .storage
            .connection
            .query_row(
                "SELECT metadata FROM memory_retrieval_debug WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| "Memory diagnostics unavailable")?
    } else {
        None
    };
    let metadata = metadata.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let retrieval=metadata.map(|m|json!({"found":m["found"].as_u64(),"passed":m["passed"].as_u64(),"context_bytes":m["context_bytes"].as_u64()}));
    let research: Option<String> = if memory.enabled && sharing && ai.web_research {
        inner.storage.connection.query_row("SELECT status FROM contextual_research WHERE goal_id IS ?1 ORDER BY id DESC LIMIT 1",[goal.as_ref().map(|g|g.id)],|r|r.get(0)).optional().map_err(|_|"Research diagnostics unavailable")?
    } else {
        None
    };
    let tavily = if !ai.web_research {
        "disabled"
    } else if !inner.status.tavily_configured {
        "key_not_configured"
    } else {
        match research.as_deref() {
            Some("authorized") => "authorized",
            Some("running") => "running",
            Some("ready") => "ready",
            Some("failed") => "failed",
            _ => "key_configured_not_search_tested",
        }
    };
    Ok(
        json!({"provider":"Claude (Anthropic)","configured_model":http::secret("ANTHROPIC_MODEL_ID").ok().as_deref().and_then(model_label),
        "configured_reasoning_model":http::secret("ANTHROPIC_MODEL_ID").ok().map(|m|http::response_model(&m)).as_deref().and_then(model_label),
        "inference_mode":if inner.engine.privacy==inner.privacy_revision {inner.engine.last.as_ref().map(|i|if goal_analysis::deliberate_operation(&i.operation){"deliberate"}else{"fast"})}else{None},
        "active_goal":goal.as_ref().map(|g|json!({"id":g.id,"label":label(&g.text)})),"active_step":step.map(|s|json!({"id":s.id,"label":label(&s.text)})),
        "work_state":crate::work_state::compute(inner)?.state,"retrieval":retrieval,"last_decision":inner.agent.diagnostics.last_decision,
        "last_skill":last_skill,"last_inference":if inner.engine.privacy==inner.privacy_revision {inner.engine.last.as_ref()}else{None},
        "observation":{"interface_text":crate::companion::screen_context::capability(inner),"screenshots_enabled":inner.companion.view.preferences.use_screenshots,"screenshot_status":if inner.vision.status.is_empty(){"not_checked"}else{inner.vision.status},"screenshot_detail":inner.vision.detail,"configured_vision_model":model_label(&crate::vision::configured_model(inner)).unwrap_or_else(||"unavailable".into()),"automatic_completion":inner.companion.view.preferences.automatic_completion,"hourly_disclosures":inner.companion.observation_calls.iter().filter(|at|at.elapsed()<Duration::from_secs(3600)).count(),"hourly_limit":crate::observation::HOURLY_LIMIT,"evidence_bytes":inner.storage.review_vault_usage().map(|(used,_)|used).unwrap_or(0),"evidence_limit_bytes":inner.storage.review_vault_usage().map(|(_,limit)|limit).unwrap_or(0),"semantic_observation":inner.companion.semantic.diagnostics()},"tavily_status":tavily,"runtime":crate::agent_runtime::diagnostics_view(inner),"health":if inner.engine.health_privacy==inner.privacy_revision {inner.engine.health.clone()}else{Health::default()},
        "configuration":{"tracking":inner.status.tracking,"ai_assistance":ai.enabled,"goal_context_sharing":ai.share_goal_context,"automatic_goal_matching":ai.automatic_goal_matching,"completion_detection":inner.companion.view.preferences.screen_task_detection,"dnd":inner.status.dnd,"paused":inner.companion.view.preferences.paused},"skills":skill_groups(),"events":inner.engine.events.iter().rev().collect::<Vec<_>>(),"demo_mode":inner.status.demo,"ai_mock":inner.status.mock_ai}),
    )
}
fn main_window(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("Open Personal AI Engine in the main Settings window".into())
    }
}
#[tauri::command(async)]
pub fn get_personal_ai_engine(
    window: WebviewWindow,
    state: State<AppState>,
) -> Result<Value, String> {
    main_window(window.label())?;
    let inner = state.inner.lock().map_err(|_| "Engine state unavailable")?;
    view(&inner)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    ok: bool,
}
fn failure(status: &'static str, error: &'static str, start: Instant) -> Health {
    Health {
        status,
        latency_ms: Some(elapsed(start)),
        provider_reported_model: None,
        error: Some(error),
        checked_at: Some(chrono::Utc::now().to_rfc3339()),
    }
}
/// One bounded request, no retries, no user context, no automatic scheduling.
async fn health_call(service: &Service<'_>, timeout: Duration) -> Health {
    let start = Instant::now();
    let schema = json!({"type":"object","additionalProperties":false,"properties":{"ok":{"type":"boolean","const":true}},"required":["ok"]});
    let payload = json!({"model":service.model,"max_tokens":1024,
        "response_format":{"type":"json_schema","json_schema":{"name":"buddy_safe_inference_health","strict":true,"schema":schema}},
        "messages":[{"role":"system","content":"Return exactly the JSON object {\"ok\":true}. No explanation or user data is needed."},{"role":"user","content":"Check structured inference."}]});
    let result = tokio::time::timeout(timeout, async {
        let (request, guard) = crate::shared_ai::prepare(
            service.client,
            reqwest::Method::POST,
            &service.endpoint,
            &service.key,
            crate::shared_ai::Auth::Claude,
        )
        .await
        .map_err(|_| {
            (
                "network_error",
                "The selected AI connection is unavailable.",
            )
        })?;
        let mut response = request
            .json(&crate::claude::to_request(&http::constrained_payload(
                &payload,
            )))
            .send()
            .await
            .map_err(|_| ("network_error", "Claude could not be reached."))?;
        guard
            .check()
            .map_err(|_| ("cancelled", "AI connection changed."))?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                401 | 403 => (
                    "access_denied",
                    "Check the saved key and model permissions.",
                ),
                402 => ("billing_required", "Provider credits are required."),
                429 => ("rate_limited", "Provider rate limit reached."),
                500..=599 => ("unavailable", "Claude is temporarily unavailable."),
                _ => (
                    "request_rejected",
                    "Check the configured model and endpoint.",
                ),
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ("network_error", "Claude response could not be read."))?
        {
            if bytes.len() + chunk.len() > 16384 {
                return Err((
                    "invalid_response",
                    "Claude health response exceeded the size limit.",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ("invalid_json", "Claude returned invalid JSON."))?;
        let value = crate::claude::from_response(value)
            .map_err(|_| ("invalid_response", "Claude returned an unexpected response."))?;
        guard
            .check()
            .map_err(|_| ("cancelled", "AI connection changed."))?;
        let text = goal_analysis::response_text(&value).map_err(|_| {
            (
                "invalid_response",
                "Claude did not complete the structured response.",
            )
        })?;
        let probe: Probe = serde_json::from_str(text).map_err(|_| {
            (
                "invalid_schema",
                "Claude did not return the required JSON Schema.",
            )
        })?;
        if !probe.ok {
            return Err((
                "invalid_schema",
                "Claude did not return the required JSON Schema.",
            ));
        }
        Ok(value["model"].as_str().and_then(model_label))
    })
    .await;
    match result {
        Ok(Ok(model)) => Health {
            status: "success",
            latency_ms: Some(elapsed(start)),
            provider_reported_model: model,
            error: None,
            checked_at: Some(chrono::Utc::now().to_rfc3339()),
        },
        Ok(Err((status, error))) => failure(status, error, start),
        Err(_) => failure(
            "timeout",
            "Claude inference timed out after the bounded wait.",
            start,
        ),
    }
}
struct HealthGuard<'a> {
    state: &'a AppState,
    finished: bool,
}
impl Drop for HealthGuard<'_> {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(mut inner) = self.state.inner.lock() {
                inner.engine.health_running = false;
                inner.engine.health.status = "cancelled";
                inner.engine.health.error = Some("The manual health check was cancelled.");
            }
        }
    }
}
fn claim_health(inner: &mut Inner, now: Instant) -> Result<u64, String> {
    if inner.engine.health_running {
        return Err("An inference health check is already running".into());
    }
    if inner
        .engine
        .health_started
        .is_some_and(|last| now.duration_since(last) < Duration::from_secs(60))
    {
        return Err("Wait 60 seconds between manual inference health checks".into());
    }
    inner.engine.health_privacy = inner.privacy_revision;
    inner.engine.health_started = Some(now);
    inner.engine.health_running = true;
    inner.engine.health = Health {
        status: "running",
        ..Default::default()
    };
    Ok(inner.privacy_revision)
}
#[tauri::command(async)]
pub async fn test_claude_inference(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Health, String> {
    main_window(window.label())?;
    let privacy = {
        let mut inner = state.inner.lock().map_err(|_| "Engine state unavailable")?;
        claim_health(&mut inner, Instant::now())?
    };
    let mut guard = HealthGuard {
        state: &state,
        finished: false,
    };
    let result = match Service::configured(&state.client) {
        Ok(service) => health_call(&service, Duration::from_secs(20)).await,
        Err(_) => failure(
            "not_configured",
            "Configure Claude before testing inference.",
            Instant::now(),
        ),
    };
    let mut inner = state.inner.lock().map_err(|_| "Engine state unavailable")?;
    let result = if inner.privacy_revision == privacy {
        result
    } else {
        failure(
            "stale",
            "Configuration or privacy changed; run a new check.",
            Instant::now(),
        )
    };
    inner.engine.health = result.clone();
    inner.engine.health_running = false;
    guard.finished = true;
    Ok(result)
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod hero_tests;
