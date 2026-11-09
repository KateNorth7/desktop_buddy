use super::*;

use wiremock::{
    matchers::{body_partial_json, header, method},
    Mock, MockServer, ResponseTemplate,
};
fn input() -> AnalysisInput {
    AnalysisInput {
        goal_id: "1".into(),
        title: "Renew passport".into(),
        description: None,
        due_at: None,
        priority: None,
        existing_steps: vec![],
        user_context_ref: None,
    }
}
fn response(content: Value) -> Value {
    json!({"choices":[{"finish_reason":"stop","message":{"content":content.to_string()}}]})
}

#[test]
fn goal_completions_keep_the_prefix_and_exclude_saved_titles() {
    let context = json!({"goals":[{"title":"Learn piano"}]});
    let result = parse_goal_completions(
        &response(json!({"completions":[
            "Learn piano", "Learn Japanese", "Learn Japanese", "Book a trip", "Le", "Learn to draw"
        ]})),
        "Le",
        &context,
    )
    .unwrap();
    assert_eq!(result, vec!["Learn Japanese", "Learn to draw"]);
    assert!(
        parse_goal_completions(&response(json!({"completions":"private"})), "Le", &context)
            .is_err()
    );
}

#[test]
fn goal_completion_context_uses_permitted_personal_knowledge_without_unrelated_scopes() {
    use crate::memory::{MemoryInput, MemoryType, Scope};
    let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    storage
        .core_add_goals(
            vec![crate::core::GoalDraft {
                title: "Learn piano".into(),
                area: "Personal".into(),
                steps: vec!["Practice scales".into()],
            }],
            "goal-completion-test",
        )
        .unwrap();
    let id = storage.core_snapshot(None).unwrap().goals[0].id;
    for (scope, text) in [
        (Scope::global(), "I work on a ceramics business"),
        (Scope::goal(id), "Private goal-specific detail"),
    ] {
        storage
            .create_memory(MemoryInput {
                scope,
                memory_type: MemoryType::ExplicitFact,
                content: text.into(),
            })
            .unwrap();
    }
    storage
        .create_memory(MemoryInput {
            scope: Scope::global(),
            memory_type: MemoryType::ExplicitPreference,
            content: "I prefer hands-on creative activities".into(),
        })
        .unwrap();
    let context = goal_completion_context(&storage, "L").unwrap().unwrap();
    assert_eq!(context["prefix"], "L");
    assert_eq!(context["goals"][0]["steps"][0]["text"], "Practice scales");
    let text = context.to_string();
    assert!(text.contains("ceramics business"));
    assert!(text.contains("hands-on creative activities"));
    assert!(!text.contains("Private goal-specific detail"));
    assert!(!text.contains("email"));
    let mut controls = storage.memory_controls().unwrap();
    controls.enabled = false;
    storage.set_memory_controls(&controls).unwrap();
    assert!(!storage.memory_controls().unwrap().enabled);
    assert!(!goal_completion_context(&storage, "L")
        .unwrap()
        .unwrap()
        .to_string()
        .contains("ceramics business"));
    assert!(goal_completion_context(&storage, "learn PIANO")
        .unwrap()
        .is_none());
    for preferences in [
        AiPreferences {
            enabled: false,
            ..Default::default()
        },
        AiPreferences {
            share_goal_context: false,
            ..Default::default()
        },
    ] {
        storage
            .write_setting("ai_assistance_preferences", &preferences)
            .unwrap();
        assert!(goal_completion_context(&storage, "L").unwrap().is_none());
    }
}
fn sources() -> Vec<SearchResult> {
    vec![SearchResult {
        title: "Official workflow".into(),
        url: "https://example.gov/workflow".into(),
        content: "Allow at least six weeks for this application.".into(),
    }]
}
fn answer() -> Value {
    json!({"result":{"improvedTitle":"Renew my passport after confirming jurisdiction","estimatedDuration":"six weeks","difficulty":"medium","suggestedSteps":[{"title":"Check required documents","sourceUrl":"https://example.gov/workflow"},{"title":"Prepare the application","sourceUrl":null},{"title":"Review and submit the application","sourceUrl":null}],"warnings":[{"title":"Prepare documents","detail":"Check the official workflow before applying.","sourceUrl":"https://example.gov/workflow"}],"resources":[{"title":"Official workflow","url":"https://example.gov/workflow","whyRelevant":"Check the required documents and order of application steps."}]},"durationEvidence":"Allow at least six weeks","difficultyEvidence":null})
}
#[test]
fn validates_context_and_does_not_invent_sources_or_precise_estimates() {
    let result = parse_analysis(&response(answer()), &input(), &sources(), "procedural").unwrap();
    assert_eq!(result.estimated_duration.as_deref(), None);
    assert_eq!(result.difficulty.as_deref(), Some("unknown"));
    let mut invalid = answer();
    invalid["durationEvidence"] = json!("Based on similar tasks");
    invalid["result"]["resources"][0]["url"] = json!("https://invented.example/requirements");
    invalid["result"]["warnings"][0]["sourceUrl"] = json!("https://invented.example/requirements");
    let result = parse_analysis(&response(invalid), &input(), &sources(), "procedural").unwrap();
    assert!(result.estimated_duration.is_none());
    assert!(result.resources.is_empty());
    assert!(result.warnings.is_empty());
    let mut bad = input();
    bad.title = "x".repeat(501);
    assert!(bad.validate().is_err());
    assert!(!safe_source("https://name:secret@example.com"));
    assert!(!safe_source("javascript:alert(1)"));
}
#[test]
fn incomplete_refused_and_malformed_provider_responses_are_rejected_without_echoing_them() {
    for value in [
        json!({"choices":[{"finish_reason":"length","message":{"content":"secret"}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"content":"secret","refusal":"no"}}]}),
        response(json!({"password":"secret"})),
    ] {
        let error = parse_analysis(&value, &input(), &[], "personal").unwrap_err();
        assert!(!error.contains("secret"));
    }
}
#[tokio::test]
async fn local_and_ambiguous_requests_do_not_call_web_search_even_if_provider_suggests_query() {
    let plan = parse_plan(&response(
        json!({"kind":"personal","query":"unneeded search"}),
    ))
    .unwrap();
    assert!(plan.query.is_none());
    let (results, notice) = research(&reqwest::Client::new(), &plan, true).await;
    assert!(results.is_empty());
    assert!(notice.is_none());
    let plan = parse_plan(&response(
        json!({"kind":"procedural","query":"passport workflow required documents"}),
    ))
    .unwrap();
    let (_, notice) = research(&reqwest::Client::new(), &plan, false).await;
    assert!(notice.unwrap().contains("off"));
}
#[tokio::test]
async fn classification_and_suggestions_use_real_http_and_only_explicit_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("x-api-key", "test-key"))
        .and(body_partial_json(
            json!({"output_config":{"format":{"schema":{"description":"buddy_research_plan"}}}}),
        ))
        .respond_with(crate::claude::test::reply(response(
            json!({"kind":"procedural","query":"passport official application workflow"}),
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"output_config":{"format":{"schema":{"description":"goal_analysis"}}}}),
        ))
        .respond_with(crate::claude::test::reply(response(answer())))
        .expect(1)
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let service = Service {
        client: &client,
        endpoint: server.uri(),
        key: "test-key".into(),
        model: "test-model".into(),
    };
    let plan = service.plan(json!({"goal":input()})).await.unwrap();
    assert_eq!(plan.kind, "procedural");
    let value = service
        .request(
            "Analyze supplied sources",
            json!({"goal":input(),"sources":sources()}),
            "goal_analysis",
            analysis_schema(),
        )
        .await
        .unwrap();
    assert_eq!(
        parse_analysis(&value, &input(), &sources(), &plan.kind)
            .unwrap()
            .resources
            .len(),
        1
    );
    for request in server.received_requests().await.unwrap() {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert!(!body.to_string().contains("test-key"));
        let shared: Value =
            serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        assert!(shared.get("profile").is_none());
        assert!(shared.get("activity").is_none());
    }
}
#[test]
fn revoked_consent_or_context_change_discards_results() {
    let storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    storage
        .write_setting(
            "ai_assistance_preferences",
            &AiPreferences {
                enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    let state = AppState::new(storage).unwrap();
    assert!(ensure_revision(&state, 0).is_ok());
    // A goal or chat edit bumps the revision but is not a consent change: the answer survives.
    state.inner.lock().unwrap().privacy_revision += 1;
    assert!(ensure_revision(&state, 0).is_ok());
    state.inner.lock().unwrap().mark_consent_change();
    assert!(ensure_revision(&state, 0).is_err());
    let inner = state.inner.lock().unwrap();
    inner
        .storage
        .write_setting(
            "ai_assistance_preferences",
            &AiPreferences {
                enabled: false,
                ..Default::default()
            },
        )
        .unwrap();
    drop(inner);
    assert!(ensure_revision(&state, 1).is_err());
}

#[test]
fn new_ai_defaults_are_enabled_but_explicit_opt_out_survives() {
    let storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let defaults = storage.ai_preferences().unwrap();
    assert!(defaults.enabled && defaults.share_goal_context && defaults.web_research);
    storage
        .write_setting(
            "ai_assistance_preferences",
            &AiPreferences {
                enabled: false,
                share_goal_context: false,
                web_research: false,
                automatic_goal_matching: false,
                voice_text_cleanup: false,
            },
        )
        .unwrap();
    assert!(!storage.ai_preferences().unwrap().enabled);
}
#[test]
fn duration_is_omitted_without_a_grounded_range() {
    let mut context = input();
    context.description = Some("Allow 2–4 hours for preparation.".into());
    let mut value = answer();
    value["result"]["estimatedDuration"] = json!("2–4 hours");
    value["durationEvidence"] = json!("Allow 2–4 hours for preparation.");
    assert_eq!(
        parse_analysis(&response(value.clone()), &context, &sources(), "procedural")
            .unwrap()
            .estimated_duration
            .as_deref(),
        Some("2–4 hours")
    );
    value["durationEvidence"] = json!("Guessing 2–4 hours is enough.");
    assert!(
        parse_analysis(&response(value), &context, &sources(), "procedural")
            .unwrap()
            .estimated_duration
            .is_none()
    );
}
#[test]
fn user_errors_never_echo_provider_diagnostics_or_credentials() {
    for error in [
        "HTTP 401: test-secret",
        "Invalid chat fields: private message",
        "HTTP 429: confidential",
        "ANTHROPIC_API_KEY=test-secret",
    ] {
        let message = user_error("test", error);
        assert!(
            !message.contains("test-secret")
                && !message.contains("HTTP")
                && !message.contains("private message")
                && !message.contains("Invalid chat fields")
        );
    }
}

#[test]
fn chat_timeouts_are_not_reported_as_unreliable_answers_or_bad_keys() {
    for error in [
        "The provider response timed out. Your saved API key has not been removed.",
        "The agent request timed out. Check the verified app state before retrying.",
        "Dialogue review timed out",
    ] {
        let message = user_error("chat", error);
        assert!(message.contains("took too long"));
        assert!(message.contains("message is saved"));
        assert!(!message.contains("more specifically"));
        assert!(!message.contains("replace"));
    }
    assert!(user_error("chat", "HTTP 503: unavailable").contains("temporarily unavailable"));
}

#[tokio::test]
async fn invalid_structured_answers_get_one_bounded_repair_without_changing_context() {
    let server = MockServer::start().await;
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = counter.clone();
    Mock::given(method("POST")).respond_with(move |_: &wiremock::Request| {
        let invalid=captured.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0;
        crate::claude::test::reply(response(json!({"message":if invalid { "" } else { "Choose the next small step." },"resources":[]})))
    }).expect(2).mount(&server).await;
    let client = reqwest::Client::new();
    let service = Service {
        client: &client,
        endpoint: server.uri(),
        key: "test-key".into(),
        model: "test-model".into(),
    };
    let result = service
        .request_validated(
            "Answer concisely",
            json!({"request":"Help me"}),
            "test_chat",
            json!({"type":"object"}),
            |value| crate::ai_chat::parse_chat(value, &[]),
        )
        .await
        .unwrap();
    assert_eq!(result.message, "Choose the next small step.");
    let requests = server.received_requests().await.unwrap();
    let before: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let after: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(before["messages"][0], after["messages"][0]);
    assert!(!after.to_string().contains("test-key"));
    assert!(crate::claude::test::system(&after)
        .as_str()
        .unwrap()
        .contains("failed validation"));
}

#[test]
fn default_coaching_fallback_preserves_explicit_models_and_the_fixed_judge() {
    for operation in ["buddy_coaching_response", "buddy_coaching_interpretation"] {
        assert!(default_interactive_fallback(
            operation,
            http::DEFAULT_PRIMARY_MODEL,
            http::DEFAULT_RESPONSE_MODEL,
            None
        ));
        assert!(!default_interactive_fallback(
            operation,
            http::DEFAULT_PRIMARY_MODEL,
            http::DEFAULT_RESPONSE_MODEL,
            Some("custom-model")
        ));
        assert!(!default_interactive_fallback(
            operation,
            "custom-model",
            http::DEFAULT_RESPONSE_MODEL,
            None
        ));
    }
    assert!(!default_interactive_fallback(
        "buddy_coaching_voice_judge",
        http::DEFAULT_PRIMARY_MODEL,
        http::DEFAULT_RESPONSE_MODEL,
        None
    ));
    assert!(
        !response_operation("buddy_coaching_voice_judge"),
        "Judge selection must ignore the response-model override"
    );
}

#[tokio::test]
async fn coaching_judge_preserves_its_model_and_does_not_retry_unavailable_or_rejected_requests() {
    for status in [401, 429, 503] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::body_partial_json(
                json!({"model":http::DEFAULT_PRIMARY_MODEL,"max_tokens":4096,"output_config":{"effort":"high","format":{"type":"json_schema","schema":{"description":"buddy_coaching_voice_judge"}}}}),
            ))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let service = Service {
            client: &client,
            endpoint: server.uri(),
            key: "fixture-key".into(),
            model: http::DEFAULT_PRIMARY_MODEL.into(),
        };
        let error = service
            .request(
                "Synthetic judge",
                json!({}),
                "buddy_coaching_voice_judge",
                object(json!({"score":{"type":"integer"}}), &["score"]),
            )
            .await
            .unwrap_err();
        assert!(error.contains(&format!("HTTP {status}")));
    }
}

#[test]
fn first_launch_enables_all_ai_features_without_starting_activity_tracking() {
    let s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let ai = s.ai_preferences().unwrap();
    assert!(ai.enabled && ai.share_goal_context && ai.web_research);
    assert!(ai.automatic_goal_matching && ai.voice_text_cleanup);
    let companion: crate::companion::engine::Preferences =
        s.read_setting("companion_preferences").unwrap().unwrap();
    assert!(
        companion.screen_task_detection
            && companion.use_screenshots
            && companion.automatic_completion
    );
    assert!(s.list_memories(Default::default()).unwrap().is_empty());
    assert!(!crate::tracking::requested(&s).unwrap());
    assert!(!s.user_settings().unwrap().onboarding.completed);
}

#[test]
fn reopening_keeps_ai_opt_outs_and_legacy_profiles_are_not_reseeded() {
    let path = std::env::temp_dir().join(format!(
        "sumi-ai-defaults-{}-{}.db",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    {
        let s = Storage::open(&path).unwrap();
        s.write_setting(
            "ai_assistance_preferences",
            &AiPreferences {
                enabled: false,
                share_goal_context: false,
                web_research: false,
                automatic_goal_matching: false,
                voice_text_cleanup: false,
            },
        )
        .unwrap();
        let companion = crate::companion::engine::Preferences {
            screen_task_detection: false,
            ..Default::default()
        };
        s.write_setting("companion_preferences", &companion)
            .unwrap();
    }
    {
        let s = Storage::open(&path).unwrap();
        let ai = s.ai_preferences().unwrap();
        assert!(!ai.enabled && !ai.automatic_goal_matching && !ai.voice_text_cleanup);
        let companion: crate::companion::engine::Preferences =
            s.read_setting("companion_preferences").unwrap().unwrap();
        assert!(
            !companion.screen_task_detection
                && !companion.use_screenshots
                && !companion.automatic_completion
        );
        // Simulate an existing older profile that has never saved AI preferences.
        s.connection.execute("DELETE FROM preferences WHERE name IN ('ai_assistance_preferences','companion_preferences')", []).unwrap();
    }
    {
        let s = Storage::open(&path).unwrap();
        let ai = s.ai_preferences().unwrap();
        assert!(!ai.automatic_goal_matching && !ai.voice_text_cleanup);
        assert!(s
            .read_setting::<crate::companion::engine::Preferences>("companion_preferences")
            .unwrap()
            .is_none());
    }
    std::fs::remove_file(path).unwrap();
}
