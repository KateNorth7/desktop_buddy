use super::*;
use crate::{core::GoalDraft, storage::Storage};
use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};
fn state() -> AppState {
    AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap()
}
fn service<'a>(state: &'a AppState, server: &MockServer) -> Service<'a> {
    Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture-secret".into(),
        model: "configured-fixture".into(),
    }
}
fn response(content: &str) -> Value {
    json!({"model":"provider-reported-fixture","choices":[{"finish_reason":"stop","message":{"content":content}}]})
}
#[tokio::test]
async fn manual_health_is_one_real_bounded_schema_request_without_user_context() {
    let state = state();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(body["model"], "configured-fixture");
            assert_eq!(body["max_tokens"], 1024);
            assert_eq!(body["output_config"]["format"]["type"], "json_schema");
            assert!(body.get("temperature").is_none());
            assert_eq!(
                body["output_config"]["format"]["schema"]["additionalProperties"],
                false
            );
            assert_eq!(
                body["output_config"]["format"]["schema"]["properties"]["ok"]["const"],
                true
            );
            assert_eq!(
                body["messages"][0]["content"],
                "Check structured inference."
            );
            assert_eq!(body["messages"].as_array().unwrap().len(), 1);
            assert!(crate::claude::test::system(&body)
                .as_str()
                .unwrap()
                .contains("Return exactly the JSON object"));
            assert!(!body.to_string().contains("fixture-secret"));
            assert!(
                body.get("memory").is_none()
                    && body.get("audio").is_none()
                    && body.get("goal").is_none()
            );
            crate::claude::test::reply(response("{\"ok\":true}"))
        })
        .expect(1)
        .mount(&server)
        .await;
    let result = health_call(&service(&state, &server), Duration::from_secs(2)).await;
    assert_eq!(result.status, "success");
    assert_eq!(
        result.provider_reported_model.as_deref(),
        Some("provider-reported-fixture")
    );
    assert!(result.latency_ms.is_some() && result.checked_at.is_some());
    assert!(result.error.is_none());
}
#[tokio::test]
async fn health_rejects_malformed_json_schema_refusal_and_truncation() {
    for (body, status) in [
        (json!({"choices":[]}), "invalid_response"),
        (response("broken"), "invalid_schema"),
        (response("{\"ok\":false}"), "invalid_schema"),
        (
            response("{\"ok\":true,\"secret\":\"fixture-secret\"}"),
            "invalid_schema",
        ),
        (
            json!({"choices":[{"finish_reason":"length","message":{"content":"{\"ok\":true}"}}]}),
            "invalid_response",
        ),
        (
            json!({"choices":[{"finish_reason":"stop","message":{"refusal":"fixture-secret"}}]}),
            "invalid_response",
        ),
    ] {
        let state = state();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(crate::claude::test::reply(body))
            .expect(1)
            .mount(&server)
            .await;
        let result = health_call(&service(&state, &server), Duration::from_secs(2)).await;
        assert_eq!(result.status, status);
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("fixture-secret"));
    }
}
#[tokio::test]
async fn health_sanitizes_http_errors_and_never_retries() {
    for (code, status) in [
        (401, "access_denied"),
        (402, "billing_required"),
        (403, "access_denied"),
        (429, "rate_limited"),
        (503, "unavailable"),
        (400, "request_rejected"),
    ] {
        let state = state();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(code)
                    .set_body_string("fixture-secret sensitive provider error"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = health_call(&service(&state, &server), Duration::from_secs(2)).await;
        assert_eq!(result.status, status);
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("fixture-secret"));
    }
}
#[tokio::test]
async fn health_handles_timeout_invalid_envelope_and_size_limit() {
    let state = state();
    for (template, status, timeout) in [
        (
            crate::claude::test::reply(response("{\"ok\":true}"))
                .set_delay(Duration::from_millis(100)),
            "timeout",
            Duration::from_millis(10),
        ),
        (
            ResponseTemplate::new(200).set_body_string("not-json"),
            "invalid_json",
            Duration::from_secs(2),
        ),
        (
            ResponseTemplate::new(200).set_body_string("x".repeat(16385)),
            "invalid_response",
            Duration::from_secs(2),
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(template)
            // The deadline includes connection setup. On a loaded Windows runner
            // it can expire before the server observes the request. Neither case
            // may retry; successful HTTP dispatch is covered by the health tests.
            .expect(if status == "timeout" { 0..=1 } else { 1..=1 })
            .mount(&server)
            .await;
        assert_eq!(
            health_call(&service(&state, &server), timeout).await.status,
            status
        );
    }
}
#[test]
fn health_single_flight_cooldown_cancellation_and_window_boundary() {
    let state = state();
    let now = Instant::now();
    {
        let mut inner = state.inner.lock().unwrap();
        claim_health(&mut inner, now).unwrap();
        assert!(claim_health(&mut inner, now)
            .unwrap_err()
            .contains("already running"));
    }
    drop(HealthGuard {
        state: &state,
        finished: false,
    });
    let mut inner = state.inner.lock().unwrap();
    assert!(!inner.engine.health_running);
    assert_eq!(inner.engine.health.status, "cancelled");
    assert!(claim_health(&mut inner, now + Duration::from_secs(59))
        .unwrap_err()
        .contains("60 seconds"));
    claim_health(&mut inner, now + Duration::from_secs(61)).unwrap();
    assert!(main_window("main").is_ok());
    assert!(main_window("buddy").is_err());
}
#[test]
fn diagnostics_are_read_only_redacted_and_hide_stale_or_disabled_memory() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner
        .storage
        .core_add_goals(
            vec![GoalDraft {
                title: "Hackathon".into(),
                area: "Work".into(),
                steps: vec!["Draft application".into()],
            }],
            "engine-test",
        )
        .unwrap();
    let id = inner.storage.core_snapshot(None).unwrap().goals[0].id;
    inner.storage.transition_goal(id, "resume").unwrap();
    inner.storage.connection.execute("INSERT OR REPLACE INTO memory_retrieval_debug(id,metadata) VALUES(1,?1)",[json!({"found":3,"passed":1,"context_bytes":57,"selected":[{"content":"RAW_ACCESSIBILITY_CANARY"}]}).to_string()]).unwrap();
    let before = inner.storage.connection.total_changes();
    let data = view(&inner).unwrap();
    assert_eq!(inner.storage.connection.total_changes(), before);
    assert_eq!(data["active_goal"]["id"], id);
    assert_eq!(data["retrieval"]["passed"], 1);
    assert!(!data.to_string().contains("RAW_ACCESSIBILITY_CANARY"));
    assert!(data.get("prompt").is_none() && data.get("key").is_none());
    assert_eq!(data["skills"].as_array().unwrap().len(), 7);
    let listed: usize = data["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["skills"].as_array().unwrap().len())
        .sum();
    assert_eq!(listed, skills::registry().len());
    inner.engine.health = Health {
        status: "success",
        ..Default::default()
    };
    inner.privacy_revision += 1;
    assert_eq!(view(&inner).unwrap()["health"]["status"], "not_checked");
    let mut controls = inner.storage.memory_controls().unwrap();
    controls.enabled = false;
    inner.storage.set_memory_controls(&controls).unwrap();
    assert!(view(&inner).unwrap()["retrieval"].is_null());
    let mut ai = inner.storage.ai_preferences().unwrap();
    ai.share_goal_context = false;
    inner
        .storage
        .write_setting("ai_assistance_preferences", &ai)
        .unwrap();
    assert!(view(&inner).unwrap()["active_goal"].is_null());
}
#[test]
fn request_cancellation_newest_request_and_privacy_staleness_are_observable() {
    let state = state();
    drop(InferenceGuard::start(&state, "buddy_agent_reasoning").unwrap());
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .engine
            .last
            .as_ref()
            .unwrap()
            .status,
        "cancelled"
    );
    let mut first = InferenceGuard::start(&state, "buddy_chat").unwrap();
    let mut second = InferenceGuard::start(&state, "buddy_contextual_search_intent").unwrap();
    second.finish("success", Some("reported".into()));
    first.finish("error", None);
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .engine
            .last
            .as_ref()
            .unwrap()
            .operation,
        "buddy_contextual_search_intent"
    );
    let mut stale = InferenceGuard::start(&state, "buddy_chat").unwrap();
    state.inner.lock().unwrap().privacy_revision += 1;
    stale.finish("success", None);
    assert!(view(&state.inner.lock().unwrap()).unwrap()["last_inference"].is_null());
    assert!(model_label("model\nsecret=token").is_none());
    assert!(model_label("claude-haiku-5-5").is_some());
}
#[tokio::test]
async fn instrumented_request_uses_existing_validation_and_records_metadata_only() {
    let state = state();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(crate::claude::test::reply(response("{\"ok\":true}")))
        .expect(1)
        .mount(&server)
        .await;
    let result = request_validated(
        &state,
        &service(&state, &server),
        "untrusted content stays out of telemetry",
        json!({"private":"RAW_CONTENT_CANARY"}),
        "buddy_chat",
        json!({"type":"object"}),
        |v| Ok(goal_analysis::response_text(v)?.to_owned()),
    )
    .await
    .unwrap();
    assert!(result.contains("true"));
    let data = view(&state.inner.lock().unwrap()).unwrap();
    assert_eq!(data["last_inference"]["status"], "success");
    assert_eq!(
        data["last_inference"]["provider_reported_model"],
        "provider-reported-fixture"
    );
    assert!(!data.to_string().contains("RAW_CONTENT_CANARY"));
}

#[test]
fn sidebar_status_is_content_free_and_configuration_does_not_claim_connectivity() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.anthropic_configured = true;
    inner.status.web_search_configured = true;
    let status = service_status_view(&inner);
    assert_eq!(status["services"][0]["status"], "not_checked");
    assert_eq!(status["services"][1]["status"], "not_checked");
    assert_eq!(status["services"][2]["status"], "not_checked");
    let mut keys: Vec<_> = status
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "configured_image_description_model",
            "configured_model",
            "configured_response_model",
            "configured_vision_model",
            "connection_mode",
            "services"
        ]
    );
    assert_eq!(
        status["configured_vision_model"],
        serde_json::json!(model_label(&crate::vision::configured_model(&inner)))
    );
    assert_eq!(
        status["configured_image_description_model"],
        serde_json::json!(model_label(&crate::vision::image_model()))
    );
    assert_eq!(status["connection_mode"], "personal");
    let serialized = status.to_string();
    for private in [
        "api_key",
        "active_goal",
        "memory",
        "accessibility",
        "prompt",
    ] {
        assert!(!serialized.contains(private));
    }
    assert!(main_window("buddy").is_err());
}
#[test]
fn sidebar_distinguishes_provider_access_from_model_access_and_expires_checks() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.anthropic_configured = true;
    inner.status.web_search_configured = true;
    begin_connection_check(&mut inner, "anthropic").unwrap();
    assert!(begin_connection_check(&mut inner, "anthropic").is_err());
    assert_eq!(
        service_status_view(&inner)["services"][0]["status"],
        "checking"
    );
    let revision = inner.privacy_revision;
    finish_connection_check(&mut inner, "anthropic", revision, true, Some(false));
    let status = service_status_view(&inner);
    assert_eq!(status["services"][0]["status"], "verified");
    assert_eq!(status["services"][1]["status"], "unavailable");
    assert!(begin_connection_check(&mut inner, "anthropic").is_err());
    assert_eq!(
        service_status_view(&inner)["services"][2]["status"],
        "verified"
    );
    inner.engine.connections[0].as_mut().unwrap().checked =
        Instant::now() - Duration::from_secs(16 * 60);
    assert_eq!(
        service_status_view(&inner)["services"][0]["status"],
        "expired"
    );
    assert_eq!(
        service_status_view(&inner)["services"][1]["status"],
        "expired"
    );
}
#[test]
fn sidebar_invalidates_checks_and_discards_results_when_credentials_or_privacy_change() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.anthropic_configured = true;
    let revision = inner.privacy_revision;
    finish_connection_check(&mut inner, "anthropic", revision, true, Some(true));
    assert_eq!(
        service_status_view(&inner)["services"][1]["status"],
        "verified"
    );
    inner.mark_consent_change();
    assert_eq!(
        service_status_view(&inner)["services"][1]["status"],
        "not_checked"
    );
    begin_connection_check(&mut inner, "anthropic").unwrap();
    finish_connection_check(&mut inner, "anthropic", revision, true, Some(true));
    // A stale completion must not clear the newer connection check.
    assert_eq!(
        service_status_view(&inner)["services"][0]["status"],
        "checking"
    );
    let current_revision = inner.privacy_revision;
    finish_connection_check(&mut inner, "anthropic", current_revision, true, Some(true));
    assert_eq!(
        service_status_view(&inner)["services"][0]["status"],
        "verified"
    );
    inner.status.mock_ai = true;
    assert_eq!(service_status_view(&inner)["services"][0]["status"], "mock");
}

#[test]
fn successful_probe_refreshes_stale_startup_configuration_flags() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.mock_ai = false;
    inner.status.anthropic_configured = false;
    inner.status.web_search_configured = false;
    let revision = inner.privacy_revision;
    finish_connection_check(&mut inner, "anthropic", revision, true, Some(true));
    assert!(inner.status.anthropic_configured && inner.status.web_search_configured);
    for service in service_status_view(&inner)["services"].as_array().unwrap() {
        assert_eq!(service["status"], "verified");
    }
}

#[test]
fn failed_or_stale_probe_cannot_restore_removed_credential_status() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.mock_ai = false;
    inner.status.anthropic_configured = false;
    inner.status.web_search_configured = false;
    let revision = inner.privacy_revision;
    finish_connection_check(&mut inner, "anthropic", revision, false, None);
    inner.mark_consent_change();
    finish_connection_check(&mut inner, "anthropic", revision, true, None);
    assert!(!inner.status.anthropic_configured && !inner.status.web_search_configured);
    assert_eq!(
        service_status_view(&inner)["services"][2]["status"],
        "not_configured"
    );
}

#[test]
fn a_probe_from_the_previous_connection_cannot_clear_the_new_connections_check() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    let old = inner.last_consent_change;
    begin_connection_check(&mut inner, "anthropic").unwrap();
    inner.mark_consent_change();
    invalidate_connections(&mut inner);
    begin_connection_check(&mut inner, "anthropic").unwrap();
    finish_connection_check(&mut inner, "anthropic", old, true, Some(true));
    assert!(inner.engine.checking_connections[0]);
    assert!(inner.engine.connections[0].is_none());
}

#[test]
fn goal_edits_do_not_invalidate_provider_authentication_or_in_flight_checks() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.anthropic_configured = true;
    inner.status.web_search_configured = true;
    let revision = inner.last_consent_change;
    finish_connection_check(&mut inner, "anthropic", revision, true, Some(true));
    // Goal and chat edits advance the domain revision, without changing consent.
    inner.privacy_revision += 1;
    assert!(begin_connection_check(&mut inner, "anthropic").is_err());
    for service in service_status_view(&inner)["services"].as_array().unwrap() {
        assert_eq!(service["status"], "verified");
    }
    inner.mark_consent_change();
    for service in service_status_view(&inner)["services"].as_array().unwrap() {
        assert_eq!(service["status"], "not_checked");
    }
}

#[test]
fn privacy_change_releases_old_probe_without_waiting_for_its_response() {
    let state = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.anthropic_configured = true;
    begin_connection_check(&mut inner, "anthropic").unwrap();
    let old = inner.privacy_revision;
    inner.mark_consent_change();
    assert_eq!(
        service_status_view(&inner)["services"][0]["status"],
        "not_checked"
    );
    begin_connection_check(&mut inner, "anthropic").unwrap();
    finish_connection_check(&mut inner, "anthropic", old, true, Some(true));
    assert_eq!(
        service_status_view(&inner)["services"][0]["status"],
        "checking"
    );
}
