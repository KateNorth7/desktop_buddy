use super::*;
use crate::{
    core::GoalDraft,
    memory::{MemoryInput, MemoryQuery, MemoryType},
    skills,
};
use wiremock::{
    matchers::{body_partial_json, method},
    Mock, MockServer, ResponseTemplate,
};
fn add(s: &mut Storage, title: &str, batch: &str) -> i64 {
    s.core_add_goals(
        vec![GoalDraft {
            title: title.into(),
            area: "Work".into(),
            steps: vec![
                "Investigate async lifetime".into(),
                "Test the implementation".into(),
            ],
        }],
        batch,
    )
    .unwrap();
    s.core_snapshot(None)
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.title == title)
        .unwrap()
        .id
}
fn state() -> (AppState, i64, String) {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let goal = add(&mut s, "Rust workspace", "research-a");
    s.core_today(goal, true, &chrono::Local::now().date_naive().to_string())
        .unwrap();
    s.transition_goal(goal, "resume").unwrap();
    let step = s.goal_plan(goal).unwrap().steps[0].id.clone();
    let state = AppState::new(s).unwrap();
    state.inner.lock().unwrap().status.mock_ai = false;
    (state, goal, step)
}
fn service<'a>(state: &'a AppState, server: &MockServer) -> Service<'a> {
    Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    }
}
fn response(v: Value) -> ResponseTemplate {
    crate::claude::test::reply(
        json!({"choices":[{"finish_reason":"stop","message":{"content":v.to_string()}}]}),
    )
}
async fn intent(server: &MockServer, n: u64) {
    Mock::given(method("POST")).and(body_partial_json(json!({"output_config":{"format":{"schema":{"description":"buddy_research_evidence"}}}})))
        .respond_with(|r:&wiremock::Request| {
            let request:Value=serde_json::from_slice(&r.body).unwrap();
            let context:Value=serde_json::from_str(request["messages"][0]["content"].as_str().unwrap()).unwrap();
            let sources=context["sources"].as_array().unwrap().iter().enumerate().map(|(index,s)| {
                let available = s["excerpts"].as_array().unwrap();
                json!({"source_index":index,"applicability":if !available.is_empty() {"applicable"}else{"unverified"},"reason":"Synthetic source","excerpt_ids":if !available.is_empty() {vec![0]}else{vec![]}})
            }).collect::<Vec<_>>();
            response(json!({"sources":sources}))
        }).mount(server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"output_config":{"format":{"schema":{"description":"buddy_contextual_search_intent"}}}}),
        ))
        .respond_with(response(
            json!({"query":"Rust async lifetime official guide"}),
        ))
        .expect(n)
        .mount(server)
        .await;
}
async fn synthesis(server: &MockServer, n: u64) {
    Mock::given(method("POST")).and(body_partial_json(json!({"output_config":{"format":{"schema":{"description":"buddy_contextual_research_result"}}}})))
        .respond_with(response(json!({"summary":"Keep borrowed data alive for the async task.","next_action":"Move an owned value into the task and run the example.","source_indices":[0]}))).expect(n).mount(server).await;
}
async fn search(server: &MockServer, results: Value, n: u64) {
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"tools":[{"name":"web_search"}]})))
        .respond_with(crate::claude::test::search_reply(results))
        .expect(n)
        .mount(server)
        .await;
}
fn sources() -> Value {
    json!([{"title":"Rust async guide","url":"https://rust-lang.org/async","content":"Owned values can outlive the borrowed scope."}])
}
fn authorize(state: &AppState, goal: i64, step: &str) -> i64 {
    state
        .inner
        .lock()
        .unwrap()
        .storage
        .queue_research(goal, Some(step.into()), "Find async lifetime guidance")
        .unwrap()
}
async fn complete(state: &AppState, goal: i64, step: &str) -> Attachment {
    let claude = MockServer::start().await;
    intent(&claude, 1).await;
    synthesis(&claude, 1).await;
    search(&claude, sources(), 1).await;
    let id = authorize(state, goal, step);
    run_with_service(state, goal, id, &service(state, &claude))
        .await
        .unwrap()
}
fn plan(snap: &skills::Snapshot, goal: i64, step: Option<&str>) -> skills::Plan {
    skills::Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions: vec![skills::Action {
            name: "search_web".into(),
            input: json!({"goal":{"goal_id":goal,"reference":null},"step":step.map(|s|json!({"step_id":s,"ordinal":null,"reference":null})),"text":"Find async lifetime guidance"}),
            instruction: Some("Search for guidance".into()),
        }],
        clarification: None,
    }
}
#[tokio::test]
async fn stuck_without_consent_never_calls_web_search_or_enqueues_network_work() {
    let (state, goal, step) = state();
    let claude = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&claude)
        .await;
    let mut inner = state.inner.lock().unwrap();
    let offer = stuck_offer(&inner, Some(goal), Some(&step)).unwrap();
    assert_eq!(offer.kind, crate::companion::engine::Kind::Research);
    assert_eq!(
        inner
            .storage
            .connection
            .query_row("SELECT COUNT(*) FROM contextual_research", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(respond_offer(&mut inner, &offer, false).unwrap().is_none());
    let memories = inner
        .storage
        .list_memories(MemoryQuery {
            scope: Some(Scope::goal(goal)),
            ..Default::default()
        })
        .unwrap();
    assert!(memories
        .iter()
        .any(|m| m.event_kind == Some(memory::EventKind::ResearchRejected)));
    assert!(respond_offer(&mut inner, &offer, true).is_err());
}
#[tokio::test]
async fn confirmation_to_web_search_to_work_context_mocked_end_to_end() {
    let (state, goal, step) = state();
    let claude = MockServer::start().await;
    intent(&claude, 1).await;
    synthesis(&claude, 1).await;
    let mut candidates = sources();
    candidates.as_array_mut().unwrap().push(json!({"title":"Another useful guide","url":"https://rust-lang.org/other","content":"Another approach"}));
    search(&claude, candidates, 1).await;
    let id = {
        let mut inner = state.inner.lock().unwrap();
        let offer = stuck_offer(&inner, Some(goal), Some(&step)).unwrap();
        respond_offer(&mut inner, &offer, true).unwrap().unwrap()
    };
    let result = run_with_service(&state, goal, id, &service(&state, &claude))
        .await
        .unwrap();
    let mut inner = state.inner.lock().unwrap();
    let context = inner.storage.work_context(goal).unwrap().unwrap();
    assert_eq!(context.context.step_id.as_deref(), Some(step.as_str()));
    assert_eq!(context.research.len(), 1);
    assert_eq!(context.research[0].research_id, id);
    assert!(context.research[0].sources[0].content.is_empty());
    let seen = inner.storage.seen_resource_keys(goal).unwrap();
    assert!(seen.contains("https://rust-lang.org/async"));
    assert!(!seen.contains("https://rust-lang.org/other")); // Uncited candidates were not shown.
    assert_eq!(
        context.context.next_action.as_deref(),
        Some(result.next_action.as_str())
    );
    assert!(!inner.storage.goal_plan(goal).unwrap().steps[0].done);
    let refs = inner
        .storage
        .action_references(inner.storage.current_conversation().unwrap())
        .unwrap();
    assert_eq!(refs.last_action.unwrap()["data"]["research_id"], id);
    assert_eq!(refs.last_referenced_step_id.as_deref(), Some(step.as_str()));
    inner.storage.research_feedback(goal, id, true).unwrap();
    assert!(inner
        .storage
        .list_memories(MemoryQuery {
            scope: Some(Scope::goal(goal)),
            ..Default::default()
        })
        .unwrap()
        .iter()
        .any(|m| m.event_kind == Some(memory::EventKind::ResearchAccepted)));
    println!(
        "RESEARCH_E2E {}",
        json!({"flow":"stuck -> offer -> explicit consent -> intent -> web search -> synthesis -> WorkContext -> helpful feedback","goal_id":goal,"step_id":step,"claude_calls":3,"tavily_calls":1,"attached_sources":result.sources.len(),"completed_step":false})
    );
}
#[tokio::test]
async fn correct_goal_step_and_only_relevant_memory_reach_claude() {
    let (state, goal, step) = state();
    let other;
    {
        let mut inner = state.inner.lock().unwrap();
        other = add(
            &mut inner.storage,
            "Unrelated payroll project",
            "research-b",
        );
        inner
            .storage
            .create_memory(MemoryInput {
                memory_type: MemoryType::ExplicitFact,
                scope: Scope::goal(other),
                content: "PRIVATE unrelated payroll context".into(),
            })
            .unwrap();
        inner
            .storage
            .create_memory(MemoryInput {
                memory_type: MemoryType::ExplicitFact,
                scope: Scope::goal(goal),
                content: "Use owned Rust values".into(),
            })
            .unwrap();
    }
    let claude = MockServer::start().await;
    intent(&claude, 1).await;
    synthesis(&claude, 1).await;
    search(&claude, sources(), 1).await;
    let id = authorize(&state, goal, &step);
    run_with_service(&state, goal, id, &service(&state, &claude))
        .await
        .unwrap();
    let requests = claude.received_requests().await.unwrap();
    for r in requests {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        if body.get("tools").is_some() {
            continue; // the web search request carries only the generated query
        }
        let text = body["messages"][0]["content"].as_str().unwrap();
        assert!(!text.contains("payroll"));
        if crate::claude::test::operation(&body) == "buddy_research_evidence" {
            assert!(!text.contains("Use owned Rust values") && !text.contains(&step));
            continue;
        }
        assert!(text.contains("Use owned Rust values"));
        assert!(text.contains(&step));
        assert!(text.contains("Rust workspace"));
    }
    let body: Value = claude
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap())
        .find(|b| b.get("tools").is_some())
        .unwrap();
    // The query is the only user content of the search request.
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["messages"][0]["content"],
        "Rust async lifetime official guide"
    );
    assert!(!body.to_string().contains("memory"));
    assert!(!body.to_string().contains(&step));
    let inner = state.inner.lock().unwrap();
    assert!(inner.storage.research_record(other, id).is_err());
    assert!(inner.storage.work_context(other).unwrap().is_none());
}
#[test]
fn chat_search_requires_confirmation_and_skill_receipt_is_backend_owned() {
    let (state, goal, step) = state();
    let mut inner = state.inner.lock().unwrap();
    let conversation = inner.storage.current_conversation().unwrap();
    let mut snap = skills::snapshot(&inner.storage, conversation).unwrap();
    snap.privacy_revision = Some(inner.privacy_revision);
    let p = plan(&snap, goal, Some(&step));
    let preview = skills::execute(
        &mut inner.storage,
        &snap,
        &p,
        "Search for guidance",
        true,
        false,
    )
    .unwrap();
    assert_eq!(preview.status, "confirmation_required");
    let preview_message = preview.fallback_message();
    assert!(preview_message.contains("Rust workspace"));
    assert!(preview_message.contains("Investigate async lifetime"));
    assert!(preview
        .message
        .unwrap()
        .contains("Only a short search query"));
    assert_eq!(
        inner
            .storage
            .connection
            .query_row("SELECT COUNT(*) FROM contextual_research", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let revision = inner.privacy_revision;
    let done = skills::pending_reply(&mut inner.storage, conversation, "Confirm", true, revision)
        .unwrap()
        .unwrap();
    assert_eq!(done.status, "executed");
    let id = done.results[0].data["research_id"].as_i64().unwrap();
    assert_eq!(
        inner.storage.research_record(goal, id).unwrap().status,
        "authorized"
    );
    assert!(
        skills::pending_reply(&mut inner.storage, conversation, "Confirm", true, revision)
            .unwrap()
            .is_none()
    );
}
#[test]
fn invalid_goal_step_and_cross_goal_step_are_rejected() {
    let (state, goal, step) = state();
    let mut inner = state.inner.lock().unwrap();
    let other = add(&mut inner.storage, "Other project", "research-other");
    // Imported plans use goal-local IDs (step-0). Give this fixture distinct IDs so
    // the test actually supplies a foreign (goal_id, step_id) pair.
    let mut other_plan = inner.storage.goal_plan(other).unwrap();
    for step in &mut other_plan.steps {
        step.id = format!("other-{}", step.id);
    }
    inner
        .storage
        .connection
        .execute(
            "UPDATE goal_plans SET payload=?1 WHERE goal_id=?2",
            params![serde_json::to_string(&other_plan).unwrap(), other],
        )
        .unwrap();
    assert!(inner
        .storage
        .queue_research(9999, None, "Public query")
        .is_err());
    assert!(inner
        .storage
        .queue_research(goal, Some("missing".into()), "Public query")
        .is_err());
    assert!(inner
        .storage
        .queue_research(other, Some(step.clone()), "Public query")
        .is_err());
    let conversation = inner.storage.current_conversation().unwrap();
    let snap = skills::snapshot(&inner.storage, conversation).unwrap();
    assert!(skills::execute(
        &mut inner.storage,
        &snap,
        &plan(&snap, other, Some(&step)),
        "Search for guidance",
        true,
        true
    )
    .is_err());
}
#[tokio::test]
async fn source_validation_and_duplicate_resources_preserve_real_destinations() {
    let (state, goal, step) = state();
    let claude = MockServer::start().await;
    state
        .inner
        .lock()
        .unwrap()
        .storage
        .save_suggestion(goal, "https://rust-lang.org/old")
        .unwrap();
    intent(&claude, 1).await;
    synthesis(&claude, 1).await;
    search(&claude,json!([
        {"title":"Old guide","url":"https://rust-lang.org/old?utm_source=mail","content":"old"},
        {"title":"Unsafe","url":"javascript:alert(1)","content":"bad"},
        {"title":"Credentials","url":"https://name:pass@example.com/","content":"bad"},
        {"title":"New guide","url":"https://rust-lang.org/new?id=2&utm_source=web","content":"New guide: move owned values into the async task."},
        {"title":"Duplicate","url":"https://rust-lang.org/new?id=2#part","content":"duplicate"}
    ]),1).await;
    let id = authorize(&state, goal, &step);
    let result = run_with_service(&state, goal, id, &service(&state, &claude))
        .await
        .unwrap();
    assert_eq!(result.sources.len(), 1);
    assert_eq!(
        result.sources[0].url,
        "https://rust-lang.org/new?id=2&utm_source=web"
    );
    let req = claude.received_requests().await.unwrap();
    let final_request = String::from_utf8(req.last().unwrap().body.clone()).unwrap();
    assert!(!final_request.contains("javascript:"));
    assert!(!final_request.contains("Old guide"));
}
#[tokio::test]
async fn duplicate_only_results_do_not_call_synthesis_or_attach() {
    let (state, goal, step) = state();
    let result = complete(&state, goal, &step).await;
    let claude = MockServer::start().await;
    intent(&claude, 1).await;
    synthesis(&claude, 0).await;
    search(&claude, sources(), 1).await;
    let id = authorize(&state, goal, &step);
    assert!(
        run_with_service(&state, goal, id, &service(&state, &claude),)
            .await
            .unwrap_err()
            .contains("No new")
    );
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .work_context(goal)
            .unwrap()
            .unwrap()
            .research[0]
            .research_id,
        result.research_id
    );
}
#[tokio::test]
async fn web_search_unavailable_has_no_synthesis_or_attached_result() {
    let (state, goal, step) = state();
    let claude = MockServer::start().await;
    intent(&claude, 1).await;
    synthesis(&claude, 0).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"tools":[{"name":"web_search"}]})))
        .respond_with(ResponseTemplate::new(401).set_body_string("PRIVATE_PROVIDER_BODY"))
        .expect(1)
        .mount(&claude)
        .await;
    let id = authorize(&state, goal, &step);
    let error = run_with_service(&state, goal, id, &service(&state, &claude))
        .await
        .unwrap_err();
    assert!(!error.contains("PRIVATE_PROVIDER_BODY"));
    let inner = state.inner.lock().unwrap();
    assert!(inner
        .storage
        .work_context(goal)
        .unwrap()
        .unwrap()
        .research
        .is_empty());
    assert_eq!(
        inner.storage.research_record(goal, id).unwrap().status,
        "failed"
    );
}
#[tokio::test]
async fn claude_unavailable_prevents_web_search() {
    let (state, goal, step) = state();
    let claude = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&claude)
        .await;
    search(&claude, sources(), 0).await;
    let id = authorize(&state, goal, &step);
    assert!(
        run_with_service(&state, goal, id, &service(&state, &claude),)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn malformed_intent_and_hallucinated_source_indices_never_attach() {
    for bad_intent in [true, false] {
        let (state, goal, step) = state();
        let claude = MockServer::start().await;
        if bad_intent {
            Mock::given(method("POST"))
                .respond_with(response(json!({"query":"api_key=must-not-leave"})))
                .expect(2)
                .mount(&claude)
                .await;
            search(&claude, sources(), 0).await;
        } else {
            intent(&claude, 1).await;
            search(&claude, sources(), 1).await;
            Mock::given(method("POST")).and(body_partial_json(json!({"output_config":{"format":{"schema":{"description":"buddy_contextual_research_result"}}}})))
                .respond_with(response(json!({"summary":"A guess","next_action":"Do a thing","source_indices":[99]}))).expect(2).mount(&claude).await;
        }
        let id = authorize(&state, goal, &step);
        assert!(
            run_with_service(&state, goal, id, &service(&state, &claude),)
                .await
                .is_err()
        );
        assert!(state
            .inner
            .lock()
            .unwrap()
            .storage
            .work_context(goal)
            .unwrap()
            .unwrap()
            .research
            .is_empty());
    }
}

#[tokio::test]
async fn synthesis_repairs_with_source_bounds_without_chat_action_instructions() {
    let (state, goal, step) = state();
    let claude = MockServer::start().await;
    intent(&claude, 1).await;
    search(&claude, sources(), 1).await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"output_config":{"format":{"schema":{"description":"buddy_contextual_research_result"}}}})))
        .respond_with(move |request:&wiremock::Request| {
            let body:Value=serde_json::from_slice(&request.body).unwrap();
            let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            assert_eq!(context["sources"][0]["source_index"],0);
            let items=&body["output_config"]["format"]["schema"]["properties"]["source_indices"]["items"];
            assert!(items.get("maximum").is_none());
            assert!(items["description"].as_str().unwrap().contains("\"maximum\":0"));
            let first=observed.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0;
            if !first {
                let instructions=crate::claude::test::system(&body).as_str().unwrap();
                assert!(!instructions.contains("Copy instruction verbatim"));
                assert!(!instructions.contains("Use only registered skills"));
            }
            response(json!({"summary":"Use an owned value for the async task.","next_action":"Try the documented example.","source_indices":[if first {99}else{0}]}))
        }).expect(2).mount(&claude).await;
    let id = authorize(&state, goal, &step);
    let result = run_with_service(&state, goal, id, &service(&state, &claude))
        .await
        .unwrap();
    assert_eq!(result.sources.len(), 1);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}
#[tokio::test]
async fn privacy_revoked_during_intent_cancels_before_web_search() {
    let (state, goal, step) = state();
    let claude = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            response(json!({"query":"Public search"}))
                .set_delay(std::time::Duration::from_millis(600)),
        )
        .expect(1)
        .mount(&claude)
        .await;
    search(&claude, sources(), 0).await;
    let id = authorize(&state, goal, &step);
    let service = service(&state, &claude);
    let request = run_with_service(&state, goal, id, &service);
    let revoke = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        state.inner.lock().unwrap().privacy_revision += 1;
    };
    let (result, ()) = tokio::join!(request, revoke);
    assert!(result.unwrap_err().contains("context changed"));
}
#[test]
fn stale_goal_step_checkpoint_consent_and_replay_are_rejected() {
    let (state, goal, step) = state();
    let mut inner = state.inner.lock().unwrap();
    let offer = stuck_offer(&inner, Some(goal), Some(&step)).unwrap();
    inner.privacy_revision += 1;
    assert!(respond_offer(&mut inner, &offer, true).is_err());
    let id = inner
        .storage
        .queue_research(goal, Some(step.clone()), "Public guidance")
        .unwrap();
    let job = prepare(&mut inner, goal, id).unwrap();
    assert!(current(&inner, &job));
    assert!(prepare(&mut inner, goal, id).is_err());
    inner
        .storage
        .connection
        .execute(
            "UPDATE work_contexts SET updated_at='changed' WHERE goal_id=?1",
            [goal],
        )
        .unwrap();
    assert!(!current(&inner, &job));
}
#[tokio::test]
async fn feedback_and_attachment_are_idempotent_and_use_shared_skill_validation() {
    let (state, goal, step) = state();
    let result = complete(&state, goal, &step).await;
    let mut inner = state.inner.lock().unwrap();
    let conversation = inner.storage.current_conversation().unwrap();
    for name in [
        "attach_research_to_context",
        "record_research_feedback",
        "record_research_feedback",
    ] {
        let snap = skills::snapshot(&inner.storage, conversation).unwrap();
        let mut input =
            json!({"goal":{"goal_id":goal,"reference":null},"research_id":result.research_id});
        if name == "record_research_feedback" {
            input["helpful"] = json!(true);
        }
        let p = skills::Plan {
            kind: "actions".into(),
            state_version: snap.version.clone(),
            actions: vec![skills::Action {
                name: name.into(),
                input,
                instruction: Some("This research helped".into()),
            }],
            clarification: None,
        };
        assert_eq!(
            skills::execute(
                &mut inner.storage,
                &snap,
                &p,
                "This research helped",
                true,
                false
            )
            .unwrap()
            .status,
            "executed"
        );
    }
    let memories = inner
        .storage
        .list_memories(MemoryQuery {
            scope: Some(Scope::goal(goal)),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        memories
            .iter()
            .filter(|m| m.event_kind == Some(memory::EventKind::ResearchAccepted))
            .count(),
        1
    );
    inner
        .storage
        .research_feedback(goal, result.research_id, false)
        .unwrap();
    assert!(inner
        .storage
        .list_memories(MemoryQuery {
            scope: Some(Scope::goal(goal)),
            ..Default::default()
        })
        .unwrap()
        .iter()
        .any(|m| m.event_kind == Some(memory::EventKind::ResearchRejected)));
    let context = inner.storage.work_context(goal).unwrap().unwrap();
    assert_eq!(context.research.len(), 1);
    let retrieved = inner
        .storage
        .retrieve_contextual_memory(
            &memory::retriever::RetrievalInput {
                active_goal_id: Some(goal),
                active_step_id: Some(step),
                ..Default::default()
            },
            &Default::default(),
            true,
        )
        .unwrap();
    assert!(retrieved.context.to_string().contains("rust-lang.org"));
}
#[tokio::test]
async fn memory_controls_and_clear_scope_cover_research_without_deleting_goals() {
    let (state, goal, step) = state();
    let result = complete(&state, goal, &step).await;
    let mut inner = state.inner.lock().unwrap();
    let mut controls = inner.storage.memory_controls().unwrap();
    controls.learn_from_feedback = false;
    inner.storage.set_memory_controls(&controls).unwrap();
    inner
        .storage
        .research_feedback(goal, result.research_id, true)
        .unwrap();
    assert!(!inner
        .storage
        .list_memories(MemoryQuery {
            scope: Some(Scope::goal(goal)),
            ..Default::default()
        })
        .unwrap()
        .iter()
        .any(|m| m.event_kind == Some(memory::EventKind::ResearchAccepted)));
    controls.enabled = false;
    inner.storage.set_memory_controls(&controls).unwrap();
    assert!(inner
        .storage
        .queue_research(goal, Some(step), "Public search")
        .is_err());
    inner
        .storage
        .clear_memory_scope(&Scope::goal(goal))
        .unwrap();
    assert!(inner
        .storage
        .research_record(goal, result.research_id)
        .is_err());
    assert!(inner
        .storage
        .core_snapshot(None)
        .unwrap()
        .goals
        .iter()
        .any(|g| g.id == goal));
}
#[tokio::test]
async fn research_survives_reopen_and_pending_network_requests_are_not_replayed() {
    let path = std::env::temp_dir().join(format!(
        "buddy-research-{}.sqlite",
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let (state, goal, step) = state();
    let result = complete(&state, goal, &step).await;
    // Back up the isolated fixture DB with VACUUM INTO; no production data is accessed.
    {
        let inner = state.inner.lock().unwrap();
        inner
            .storage
            .queue_research(goal, Some(step), "Another public query")
            .unwrap();
        inner
            .storage
            .connection
            .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
    }
    let s = Storage::open(&path).unwrap();
    assert_eq!(
        s.work_context(goal).unwrap().unwrap().research[0].research_id,
        result.research_id
    );
    assert_eq!(
        s.connection
            .query_row(
                "SELECT COUNT(*) FROM contextual_research WHERE status IN ('authorized','running')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    s.clear_personal_memory().unwrap();
    assert_eq!(
        s.connection
            .query_row("SELECT COUNT(*) FROM contextual_research", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(s);
    let _ = std::fs::remove_file(path);
}
#[test]
fn expired_consent_and_mock_ai_cannot_search() {
    let (state, goal, step) = state();
    let mut inner = state.inner.lock().unwrap();
    let id = inner
        .storage
        .queue_research(goal, Some(step.clone()), "Public query")
        .unwrap();
    inner
        .storage
        .connection
        .execute(
            "UPDATE contextual_research SET expires_at=0 WHERE id=?1",
            [id],
        )
        .unwrap();
    assert!(prepare(&mut inner, goal, id).is_err());
    let id = inner
        .storage
        .queue_research(goal, Some(step), "Public query")
        .unwrap();
    inner.status.mock_ai = true;
    assert!(prepare(&mut inner, goal, id)
        .err()
        .unwrap()
        .contains("AI_MOCK"));
}

#[tokio::test]
async fn tight_memory_budget_preserves_checkpoint_ahead_of_optional_research() {
    let (state, goal, step) = state();
    complete(&state, goal, &step).await;
    let inner = state.inner.lock().unwrap();
    let input = memory::retriever::RetrievalInput {
        active_goal_id: Some(goal),
        active_step_id: Some(step),
        ..Default::default()
    };
    let full = inner
        .storage
        .retrieve_contextual_memory(&input, &Default::default(), true)
        .unwrap();
    let mut without_research = full.context.clone();
    without_research["work_context"]["research"] = json!([]);
    without_research["memories"] = json!([]);
    let limit = serde_json::to_vec(&without_research).unwrap().len();
    let budget = memory::retriever::MemoryBudget {
        maximum_context_size: limit,
        ..Default::default()
    };
    let result = inner
        .storage
        .retrieve_contextual_memory(&input, &budget, true)
        .unwrap();
    assert!(result.context["work_context"]["summary"].is_string());
    assert_eq!(result.context["work_context"]["research"], json!([]));
    assert!(serde_json::to_vec(&result.context).unwrap().len() <= limit);
}

#[test]
fn research_additive_migration_is_idempotent_and_preserves_existing_context() {
    let (state, goal, _) = state();
    let inner = state.inner.lock().unwrap();
    let before = inner
        .storage
        .work_context(goal)
        .unwrap()
        .unwrap()
        .context
        .summary;
    inner
        .storage
        .connection
        .execute_batch("DROP TABLE contextual_research")
        .unwrap();
    inner.storage.initialize_personal_memory().unwrap();
    inner.storage.initialize_personal_memory().unwrap();
    assert_eq!(
        inner
            .storage
            .work_context(goal)
            .unwrap()
            .unwrap()
            .context
            .summary,
        before
    );
    assert!(inner.storage.context_research(1).unwrap().is_empty());
}

#[tokio::test]
async fn research_preserves_waiting_dependency_and_does_not_confirm_completion() {
    let (state, goal, step) = state();
    {
        let mut inner = state.inner.lock().unwrap();
        let old = inner.storage.work_context(goal).unwrap().unwrap();
        let mut input = old.context;
        input.status = ContextStatus::Waiting;
        input.waiting_for = Some("Reviewer approval".into());
        inner
            .storage
            .save_work_context(input, Some(old.updated_at))
            .unwrap();
    }
    complete(&state, goal, &step).await;
    let inner = state.inner.lock().unwrap();
    let context = inner.storage.work_context(goal).unwrap().unwrap();
    assert_eq!(context.context.status, ContextStatus::Waiting);
    assert_eq!(
        context.context.waiting_for.as_deref(),
        Some("Reviewer approval")
    );
    assert!(context.last_result.unwrap().starts_with("Research:"));
    assert!(!inner.storage.goal_plan(goal).unwrap().steps[0].done);
}

#[test]
fn goal_only_search_uses_real_current_step_and_expired_plan_does_not_execute() {
    let (state, goal, step) = state();
    let mut inner = state.inner.lock().unwrap();
    let conversation = inner.storage.current_conversation().unwrap();
    let snap = skills::snapshot(&inner.storage, conversation).unwrap();
    let p = plan(&snap, goal, None);
    let done = skills::execute(
        &mut inner.storage,
        &snap,
        &p,
        "Search for guidance",
        true,
        true,
    )
    .unwrap();
    assert_eq!(done.results[0].step_id.as_deref(), Some(step.as_str()));
    inner
        .storage
        .connection
        .execute("UPDATE contextual_research SET status='failed'", [])
        .unwrap();
    let snap = skills::snapshot(&inner.storage, conversation).unwrap();
    let p = plan(&snap, goal, Some(&step));
    inner.privacy_revision += 1;
    let mut changed = inner.storage.goal_plan(goal).unwrap();
    changed.steps[0].text = "Changed step".into();
    inner
        .storage
        .core_save_goal("Rust workspace", "Work", changed)
        .unwrap();
    assert!(skills::execute(
        &mut inner.storage,
        &snap,
        &p,
        "Search for guidance",
        true,
        true
    )
    .is_err());
}

#[test]
fn consent_is_bound_to_context_before_the_worker_starts() {
    for change in ["plan", "focus", "checkpoint", "conversation"] {
        let (state, goal, step) = state();
        let mut inner = state.inner.lock().unwrap();
        let id = inner
            .storage
            .queue_research(goal, Some(step), "Public guidance")
            .unwrap();
        match change {
            "plan" => {
                let mut plan = inner.storage.goal_plan(goal).unwrap();
                plan.steps[0].text = "Different information gap".into();
                inner
                    .storage
                    .core_save_goal("Rust workspace", "Work", plan)
                    .unwrap();
            }
            "focus" => {
                let other = add(&mut inner.storage, "Other lane", "other-lane");
                inner.storage.transition_goal(other, "resume").unwrap();
            }
            "checkpoint" => {
                inner
                    .storage
                    .connection
                    .execute(
                        "UPDATE work_contexts SET updated_at='changed' WHERE goal_id=?1",
                        [goal],
                    )
                    .unwrap();
            }
            _ => {
                inner.storage.create_conversation().unwrap();
            }
        }
        assert!(
            prepare(&mut inner, goal, id).is_err(),
            "stale consent after {change}"
        );
    }
}
