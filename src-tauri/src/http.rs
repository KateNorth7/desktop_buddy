use serde_json::Value;
use std::time::Duration;

/// Background reasoning, matching and auxiliary operations.
pub const DEFAULT_PRIMARY_MODEL: &str = crate::claude::DEFAULT_MODEL;
/// Interactive dialogue, authorization/review and natural replies. Both roles use
/// the same Claude model unless `ANTHROPIC_RESPONSE_MODEL_ID` selects another one.
pub const DEFAULT_RESPONSE_MODEL: &str = crate::claude::DEFAULT_MODEL;
pub(crate) type Preflight<'a> = &'a (dyn Fn() -> Result<(), String> + Send + Sync);
fn resolve_response_model(primary: &str, response_override: Option<&str>) -> String {
    response_override
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(primary)
        .into()
}
pub fn response_model(primary: &str) -> String {
    if crate::shared_ai::mode() == crate::shared_ai::ConnectionMode::BuiltIn {
        return DEFAULT_RESPONSE_MODEL.into();
    }
    resolve_response_model(
        primary,
        std::env::var("ANTHROPIC_RESPONSE_MODEL_ID").ok().as_deref(),
    )
}

pub fn status_error(status: u16) -> String {
    let advice = match status {
        400 => "The provider rejected the request. Check model and endpoint compatibility.",
        401 => "The API key was rejected. Replace it in Settings.",
        403 => "This key does not have permission. Check provider access and account settings.",
        404 => "The endpoint or model was not found. Check provider configuration.",
        402 => "Provider billing or credits are required. Check your provider account.",
        429 => "Provider quota or rate limit reached. Check your limits and try again later.",
        500..=599 => "The provider is temporarily unavailable. Try again later.",
        _ => "The provider could not complete the request.",
    };
    format!("HTTP {status}: {advice}")
}

pub async fn post_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
) -> Result<Value, String> {
    post_json_with_timeout(
        client,
        url,
        key,
        payload,
        Duration::from_secs(20),
        true,
        true,
    )
    .await
}

/// Interactive reasoning can outlast the short background/search timeout. Do not
/// resubmit a timed-out generation: the provider may still be processing it.
pub(crate) async fn post_reasoning_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
) -> Result<Value, String> {
    post_json_with_timeout(
        client,
        url,
        key,
        payload,
        Duration::from_secs(60),
        false,
        true,
    )
    .await
}

/// Background screen classification is a single, bounded disclosure. A privacy
/// change must not be followed by an automatic retry of an earlier screen packet.
pub(crate) async fn post_background_once_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
) -> Result<Value, String> {
    post_json_with_timeout(
        client,
        url,
        key,
        payload,
        Duration::from_secs(30),
        false,
        false,
    )
    .await
}

/// Evaluation uses its explicitly supplied judge model and one request only.
/// Never restart a timed-out generation or silently change the judge model.
pub(crate) async fn post_evaluation_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
) -> Result<Value, String> {
    post_json_with_timeout(
        client,
        url,
        key,
        payload,
        Duration::from_secs(60),
        false,
        false,
    )
    .await
}

/// An interactive response proposes data only; it cannot execute a skill. If the
/// default reasoning model stalls, try the configured default primary model once
/// with the exact same messages and schema. Never replay an executed action.
pub(crate) async fn post_default_interactive_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
    budget: Duration,
    fallback_budget: Duration,
    preflight: Option<Preflight<'_>>,
) -> Result<Value, String> {
    if let Some(check) = preflight {
        check()?;
    }
    let first = tokio::time::timeout(
        budget,
        post_json_with_timeout(client, url, key, payload, budget, false, false),
    )
    .await;
    match first {
        Ok(Ok(value)) => return Ok(value),
        Ok(Err(error))
            if !error.contains("timed out")
                && !error.contains("could not be reached")
                && !error.contains("HTTP 5") =>
        {
            return Err(error)
        }
        _ => {}
    }
    if let Some(check) = preflight {
        check()?;
    }
    let mut fallback = payload.clone();
    fallback["model"] = serde_json::json!(DEFAULT_PRIMARY_MODEL);
    tokio::time::timeout(
        fallback_budget,
        post_json_with_timeout(client, url, key, &fallback, fallback_budget, false, false),
    )
    .await
    .map_err(|_| {
        "The AI response models timed out. Your saved API key has not been removed.".to_string()
    })?
}

async fn post_json_with_timeout(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
    timeout: Duration,
    retry_timeouts: bool,
    retry_transient: bool,
) -> Result<Value, String> {
    let payload = constrained_payload(payload);
    // Chat requests leave this process in the Claude wire format; search payloads
    // are sent as built. Responses come back in the crate's internal envelope.
    let chat = crate::claude::is_chat(&payload);
    let (body, auth) = if chat {
        (
            crate::claude::to_request(&payload),
            crate::shared_ai::Auth::Claude,
        )
    } else {
        (payload, crate::shared_ai::Auth::Bearer)
    };
    let mut last = "Service unavailable".to_string();
    let attempts = if retry_transient && key != crate::shared_ai::SHARED_CREDENTIAL {
        3
    } else {
        1
    };
    for attempt in 0..attempts {
        let (request, guard) =
            crate::shared_ai::prepare(client, reqwest::Method::POST, url, key, auth).await?;
        guard.check()?;
        let response = request.json(&body).timeout(timeout).send().await;
        guard.check()?;
        match response {
            Ok(response) if response.status().is_success() => {
                let result = response.json().await.map_err(|error| {
                    if error.is_timeout() {
                        "The provider response timed out. Your saved API key has not been removed."
                            .into()
                    } else {
                        "Service returned invalid JSON".into()
                    }
                });
                guard.check()?;
                return if chat {
                    result.and_then(crate::claude::from_response)
                } else {
                    result
                };
            }
            Ok(response) => {
                let status = response.status();
                last = status_error(status.as_u16());
                if !retry_transient || (status.as_u16() != 429 && !status.is_server_error()) {
                    return Err(last);
                }
            }
            Err(error) if error.is_timeout() && !retry_timeouts => {
                return Err(
                    "The provider response timed out. Your saved API key has not been removed."
                        .into(),
                );
            }
            Err(_) => last = "Service could not be reached or timed out".into(),
        }
        if attempt + 1 < attempts {
            tokio::time::sleep(Duration::from_millis(500 * (1 << attempt))).await;
        }
    }
    Err(last)
}
/// Keep interactive output within its budget: structured requests that do not ask
/// for deliberate reasoning run at low effort so hidden reasoning cannot consume
/// the action budget. Search payloads pass through unchanged.
pub(crate) fn constrained_payload(payload: &Value) -> Value {
    let mut result = payload.clone();
    if crate::claude::is_chat(payload)
        && payload["response_format"]["type"] == "json_schema"
        && payload["reasoning_effort"].is_null()
    {
        result["reasoning_effort"] = serde_json::json!("low");
    }
    result
}
pub fn endpoint(name: &str) -> Result<String, String> {
    let value = secret(name)?;
    let url = reqwest::Url::parse(&value).map_err(|_| format!("Invalid {name}"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(format!(
            "{name} must be an HTTPS endpoint without credentials"
        ));
    }
    Ok(value)
}
pub fn secret(name: &str) -> Result<String, String> {
    if crate::shared_ai::mode() == crate::shared_ai::ConnectionMode::BuiltIn {
        match name {
            "ANTHROPIC_API_KEY" => return Ok(crate::shared_ai::SHARED_CREDENTIAL.into()),
            "ANTHROPIC_API_URL" => return crate::shared_ai::endpoint(name),
            "ANTHROPIC_MODEL_ID" => return Ok(DEFAULT_PRIMARY_MODEL.into()),
            "ANTHROPIC_VISION_MODEL_ID" => return Ok(DEFAULT_RESPONSE_MODEL.into()),
            _ => {}
        }
    }
    // Test providers use synthetic environment keys, never the user's vault.
    if !cfg!(test) && matches!(name, "ANTHROPIC_API_KEY") {
        if let Some(key) = crate::credentials::read(name)? {
            return Ok(key);
        }
    }
    std::env::var(name)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| match name {
            "ANTHROPIC_API_URL" => Some(crate::claude::API_URL.into()),
            "ANTHROPIC_MODEL_ID" => Some(DEFAULT_PRIMARY_MODEL.into()),
            _ => None,
        })
        .ok_or_else(|| format!("Configure {name} in Settings"))
}
pub fn configured(name: &str) -> bool {
    if crate::shared_ai::mode() == crate::shared_ai::ConnectionMode::BuiltIn
        && matches!(name, "ANTHROPIC_API_KEY")
    {
        return crate::shared_ai::configured();
    }
    if !cfg!(test) && matches!(name, "ANTHROPIC_API_KEY") {
        return std::env::var(name)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
            || crate::credentials::exists(name).unwrap_or(false);
    }
    secret(name).is_ok()
}
pub fn enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|s| s.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// A single observation request, cancelled locally when foreground or consent changes.
pub(crate) async fn post_guarded_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
    check: Preflight<'_>,
) -> Result<Value, String> {
    check()?;
    let request = post_background_once_json(client, url, key, payload);
    tokio::pin!(request);
    let mut watch = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            result=&mut request=>{check()?;return result;}
            _=watch.tick()=>{check()?;}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, Request, ResponseTemplate,
    };
    fn chat(model: &str) -> Value {
        serde_json::json!({"model":model,"max_tokens":8192,
            "messages":[{"role":"system","content":"Existing instructions"},{"role":"user","content":"Synthetic request"}],
            "response_format":{"type":"json_schema","json_schema":{"name":"synthetic_operation","strict":true,"schema":{"type":"object","additionalProperties":false,"properties":{"ok":{"type":"boolean"}},"required":["ok"]}}}})
    }
    fn envelope(content: &str) -> Value {
        serde_json::json!({"model":"claude-reported","choices":[{"finish_reason":"stop","message":{"content":content}}]})
    }
    #[test]
    fn constrained_structured_requests_do_not_spend_action_budget_on_hidden_reasoning() {
        let payload = chat("claude-opus-5-5");
        assert_eq!(constrained_payload(&payload)["reasoning_effort"], "low");
        let mut explicit = payload.clone();
        explicit["reasoning_effort"] = serde_json::json!("high");
        assert_eq!(constrained_payload(&explicit), explicit);
        let prose = serde_json::json!({"model":"claude-opus-5-5","messages":[{"role":"user","content":"Write"}]});
        assert_eq!(constrained_payload(&prose), prose);
        let search = serde_json::json!({"query":"public docs"});
        assert_eq!(constrained_payload(&search), search);
    }
    #[test]
    fn response_model_defaults_and_explicit_overrides_are_predictable() {
        assert_eq!(
            resolve_response_model(DEFAULT_PRIMARY_MODEL, None),
            DEFAULT_RESPONSE_MODEL
        );
        assert_eq!(resolve_response_model("custom", None), "custom");
        assert_eq!(resolve_response_model("custom", Some("")), "custom");
        assert_eq!(
            resolve_response_model("custom", Some("claude-sonnet-5-5")),
            "claude-sonnet-5-5"
        );
        assert!(crate::goal_analysis::response_operation("buddy_skill_plan"));
        assert!(crate::goal_analysis::response_operation(
            "buddy_coaching_interpretation"
        ));
        assert!(crate::goal_analysis::response_operation(
            "buddy_coaching_response"
        ));
        assert!(crate::goal_analysis::response_operation(
            "buddy_chat_research_authorization"
        ));
        assert!(!crate::goal_analysis::response_operation(
            "buddy_agent_reasoning"
        ));
        assert!(!crate::goal_analysis::response_operation(
            "completion_detection"
        ));
        assert!(crate::goal_analysis::response_operation(
            "buddy_dialogue_authorization"
        ));
        assert!(crate::goal_analysis::response_operation(
            "buddy_dialogue_response"
        ));
        assert!(crate::goal_analysis::response_operation(
            "buddy_dialogue_review"
        ));
    }
    #[tokio::test]
    async fn chat_requests_use_the_claude_wire_format_and_return_the_internal_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "synthetic-key"))
            .and(header("anthropic-version", crate::claude::API_VERSION))
            .respond_with(|request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert!(request.headers.get("authorization").is_none());
                assert_eq!(body["model"], "claude-opus-5-5");
                assert_eq!(body["system"][0]["text"], "Existing instructions");
                assert_eq!(body["messages"][0]["content"], "Synthetic request");
                assert_eq!(body["output_config"]["effort"], "low");
                assert_eq!(body["output_config"]["format"]["type"], "json_schema");
                assert_eq!(
                    body["output_config"]["format"]["schema"]["description"],
                    "synthetic_operation"
                );
                assert!(body.get("response_format").is_none());
                assert!(body.get("temperature").is_none());
                crate::claude::test::reply(envelope("{\"ok\":true}"))
            })
            .expect(1)
            .mount(&server)
            .await;
        let result = post_json(
            &reqwest::Client::new(),
            &format!("{}/v1/messages", server.uri()),
            "synthetic-key",
            &chat("claude-opus-5-5"),
        )
        .await
        .unwrap();
        assert_eq!(result["choices"][0]["finish_reason"], "stop");
        assert_eq!(result["choices"][0]["message"]["content"], "{\"ok\":true}");
        assert_eq!(result["model"], "claude-reported");
    }
    #[tokio::test]
    async fn unexpected_chat_responses_are_rejected_without_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            post_json(
                &reqwest::Client::new(),
                &server.uri(),
                "synthetic-key",
                &chat("claude-opus-5-5"),
            )
            .await
            .unwrap_err(),
            "Service returned an unexpected response"
        );
    }
    #[tokio::test]
    async fn reasoning_request_can_outlast_the_clients_background_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_json(serde_json::json!({"ok":true})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(20))
            .build()
            .unwrap();
        let response = post_reasoning_json(
            &client,
            &server.uri(),
            "test",
            &serde_json::json!({"model":"test-model"}),
        )
        .await
        .unwrap();
        assert_eq!(response["ok"], true);
    }
    #[tokio::test]
    async fn timed_out_reasoning_is_not_resubmitted_or_reported_as_a_bad_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(300))
                    .set_body_json(serde_json::json!({"ok":true})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = post_json_with_timeout(
            &reqwest::Client::new(),
            &server.uri(),
            "secret-canary",
            &serde_json::json!({"model":"test-model"}),
            Duration::from_millis(100),
            false,
            true,
        )
        .await
        .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(error.contains("saved API key has not been removed"));
        assert!(!error.contains("secret-canary"));
    }
    #[tokio::test]
    async fn retries_transient_errors_twice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3)
            .mount(&server)
            .await;
        assert!(post_json(
            &reqwest::Client::new(),
            &server.uri(),
            "test",
            &serde_json::json!({})
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn does_not_retry_authentication_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            post_json(
                &reqwest::Client::new(),
                &server.uri(),
                "test",
                &serde_json::json!({})
            )
            .await
            .unwrap_err(),
            status_error(401)
        );
    }
    #[tokio::test]
    async fn stalled_default_planning_retries_once_with_identical_schema_and_messages() {
        let server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        Mock::given(method("POST"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["messages"][0]["content"], "Synthetic request");
                assert_eq!(body["system"][0]["text"], "Existing instructions");
                assert_eq!(body["output_config"]["format"]["type"], "json_schema");
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_secs(5))
                        .set_body_json(serde_json::json!({"late":true}))
                } else {
                    assert_eq!(body["model"], DEFAULT_PRIMARY_MODEL);
                    crate::claude::test::reply(envelope("{\"ok\":true}"))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let result = post_default_interactive_json(
            &reqwest::Client::new(),
            &server.uri(),
            "fixture",
            &chat(DEFAULT_RESPONSE_MODEL),
            Duration::from_millis(300),
            Duration::from_millis(300),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result["choices"][0]["message"]["content"], "{\"ok\":true}");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn planning_does_not_fail_over_rejected_keys_quota_or_invalid_responses() {
        for status in [400, 401, 402, 403, 404, 429] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            let payload = serde_json::json!({"model":DEFAULT_RESPONSE_MODEL});
            let error = post_default_interactive_json(
                &reqwest::Client::new(),
                &server.uri(),
                "fixture",
                &payload,
                Duration::from_millis(300),
                Duration::from_millis(300),
                None,
            )
            .await
            .unwrap_err();
            assert!(error.contains(&format!("HTTP {status}")));
        }
    }
    #[tokio::test]
    async fn planning_fallback_stops_after_two_attempts_and_rejects_invalid_json_without_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(5))
                    .set_body_json(serde_json::json!({"late":true})),
            )
            .expect(2)
            .mount(&server)
            .await;
        assert!(post_default_interactive_json(
            &reqwest::Client::new(),
            &server.uri(),
            "fixture",
            &serde_json::json!({"model":DEFAULT_RESPONSE_MODEL}),
            Duration::from_millis(300),
            Duration::from_millis(300),
            None,
        )
        .await
        .unwrap_err()
        .contains("timed out"));
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("broken-json"))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            post_default_interactive_json(
                &reqwest::Client::new(),
                &server.uri(),
                "fixture",
                &serde_json::json!({"model":DEFAULT_RESPONSE_MODEL}),
                Duration::from_millis(300),
                Duration::from_millis(300),
                None,
            )
            .await
            .unwrap_err(),
            "Service returned invalid JSON"
        );
    }
    #[tokio::test]
    async fn interactive_fallback_rechecks_consent_before_another_disclosure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_json(serde_json::json!({"late":true})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let checks = AtomicUsize::new(0);
        let check = || {
            if checks.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(())
            } else {
                Err("Coaching context or privacy changed".to_string())
            }
        };
        let error = post_default_interactive_json(
            &reqwest::Client::new(),
            &server.uri(),
            "fixture",
            &serde_json::json!({"model":DEFAULT_RESPONSE_MODEL}),
            Duration::from_millis(300),
            Duration::from_millis(300),
            Some(&check),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "Coaching context or privacy changed");
        assert_eq!(checks.load(Ordering::SeqCst), 2);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn reasoning_fallback_has_a_separate_budget_and_keeps_the_requested_effort() {
        let server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        Mock::given(method("POST"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["output_config"]["effort"], "low");
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(503)
                } else {
                    crate::claude::test::reply(envelope("{\"ok\":true}"))
                        .set_delay(Duration::from_millis(80))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let mut payload = chat(DEFAULT_RESPONSE_MODEL);
        payload["reasoning_effort"] = serde_json::json!("low");
        let result = post_default_interactive_json(
            &reqwest::Client::new(),
            &server.uri(),
            "fixture",
            &payload,
            Duration::from_millis(20),
            Duration::from_millis(300),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result["choices"][0]["finish_reason"], "stop");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}
