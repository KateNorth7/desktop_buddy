use super::*;
use crate::dialogue_benchmark::{fixture, Case};
use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};

#[test]
fn unverified_read_receipt_is_not_a_user_answer_but_mutation_receipts_are_preserved() {
    let mut execution = skills::Execution {
        status: "executed",
        results: vec![skills::SkillResult {
            skill: "inspect_conversation".into(),
            goal_id: None,
            step_id: None,
            data: json!({}),
        }],
        message: None,
    };
    assert!(execution
        .fallback_message()
        .contains("Read retained messages"));
    assert!(unverified_reply(Some(&execution))
        .message
        .contains("could not verify"));
    execution.results[0].skill = "create_goal".into();
    execution.results[0].data = json!({"title":"Synthetic goal"});
    assert!(unverified_reply(Some(&execution))
        .message
        .contains("Created"));
    execution.status = "confirmation_required";
    assert!(unverified_reply(Some(&execution))
        .message
        .contains("Proposed changes only"));
}

#[tokio::test]
async fn web_search_authorization_checks_the_request_without_sending_workspace_or_memory() {
    for authorized in [true, false] {
        let (state, _) = fixture(&case()).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |r: &Request| {
                let (b, c) = body(r);
                assert_eq!(
                    crate::claude::test::operation(&b),
                    "buddy_chat_research_authorization"
                );
                assert_eq!(c.as_object().unwrap().len(), 3);
                assert!(!c.to_string().contains("PRIVATE WORKSPACE"));
                assert!(!c.to_string().contains("PRIVATE MEMORY"));
                response(json!({"authorized":authorized}))
            })
            .expect(1)
            .mount(&server)
            .await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let context = json!({"latest_request":"Find the official Tauri deployment documentation", "conversation":[],
            "state":"PRIVATE WORKSPACE", "personal_memory":"PRIVATE MEMORY"});
        assert_eq!(
            authorize_web_query(
                &state,
                &service,
                &context,
                "Tauri 2 macOS deployment target official documentation"
            )
            .await
            .is_ok(),
            authorized
        );
    }
}

#[tokio::test]
async fn malformed_web_authorization_fails_closed() {
    let (state, _) = fixture(&case()).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response(json!({"authorized":"yes"})))
        .expect(2)
        .mount(&server)
        .await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    assert!(authorize_web_query(
        &state,
        &service,
        &json!({"latest_request":"Read my goal","conversation":[]}),
        "unrequested search"
    )
    .await
    .is_err());
}

#[tokio::test]
async fn history_read_does_not_itself_authorize_web_search() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let (state, _) = fixture(&case()).unwrap();
    state
        .inner
        .lock()
        .unwrap()
        .storage
        .write_setting(
            "ai_assistance_preferences",
            &goal_analysis::AiPreferences {
                web_research: true,
                ..Default::default()
            },
        )
        .unwrap();
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    let plans = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .respond_with(move |r: &Request| {
            let (b, c) = body(r);
            match crate::claude::test::operation(&b).as_str().unwrap() {
                "buddy_skill_plan" if plans.fetch_add(1, Ordering::SeqCst) == 0 => {
                    response(action(
                        &c,
                        "inspect_conversation",
                        json!({"query":"","before_message_id":null}),
                        "read",
                        true,
                    ))
                }
                "buddy_skill_plan" => {
                    let mut a = answer(&c, "I will search.");
                    a["web_query"] = json!("unrequested search");
                    response(a)
                }
                "buddy_chat_research_authorization" => response(json!({"authorized":false})),
                name => panic!("Unexpected request: {name}"),
            }
        })
        .expect(3)
        .mount(&server)
        .await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let error=run(&json!({"latest_request":"Read what we said here. Do not search.","conversation":[],"natural_reply":true}),conversation,&state,0,false,&service).await.err().unwrap();
    assert!(error.contains("No search was sent"), "{error}");
}

#[tokio::test]
async fn research_disabled_after_a_history_read_sends_no_search_and_reports_the_setting() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let (state, _) = fixture(&case()).unwrap();
    let conversation = {
        let inner = state.inner.lock().unwrap();
        inner
            .storage
            .write_setting(
                "ai_assistance_preferences",
                &goal_analysis::AiPreferences {
                    web_research: false,
                    ..Default::default()
                },
            )
            .unwrap();
        inner.storage.current_conversation().unwrap()
    };
    let server = MockServer::start().await;
    let plans = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST")).respond_with(move |r: &Request| {
        let (b,c)=body(r);
        match crate::claude::test::operation(&b).as_str().unwrap_or("prose") {
            "buddy_skill_plan" if plans.fetch_add(1,Ordering::SeqCst)==0 =>
                response(action(&c,"inspect_conversation",json!({"query":"Tauri","before_message_id":null}),"read",true)),
            "buddy_skill_plan" => { let mut a=answer(&c,"Search requested.");a["web_query"]=json!("Tauri macOS deployment documentation");response(a) },
            "prose" => crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":"Web research is off; I have no retrieved sources."}}]})),
            "buddy_dialogue_review" => response(json!({"verdict":"approved","request_mode":"answer","message":""})),
            name => panic!("Unexpected external request: {name}"),
        }
    }).expect(2).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let result=run(&json!({"latest_request":"Find official documentation for that topic","conversation":[],"natural_reply":true}),conversation,&state,0,false,&service).await.unwrap();
    assert!(!result.committed);
    assert!(result.reply.message.contains("Web research is off"));
    assert!(result.reply.resources.is_empty());
}
fn case() -> Case {
    Case {
        id: "fixture-en".into(),
        language: "en".into(),
        kind: "help".into(),
        request: "Help with that assignment".into(),
        history: vec![json!({"role":"user","text":"Help me with Psychology assignment"})],
    }
}
fn envelope(raw: Value) -> Value {
    json!({"choices":[{"finish_reason":"stop","message":{"content":raw.to_string()}}]})
}
fn answer(context: &Value, message: &str) -> Value {
    json!({"situation":"Answer using stored facts","decision":"stay_silent","proposed_actions":[],"memory_updates":[],"state_version":context["state"]["state_version"],"clarification":null,"intent":{"mode":"answer","operations":[]},"message":message,"web_query":null,"compose_response":false})
}
fn action(context: &Value, name: &str, input: Value, mode: &str, compose: bool) -> Value {
    json!({"situation":"Use the requested skill","decision":"execute_explicit_user_action","proposed_actions":[{"name":name,"input":input,"instruction":context["latest_request"]}],"memory_updates":[],"state_version":context["state"]["state_version"],"clarification":null,"intent":{"mode":mode,"operations":[name]},"message":"","web_query":null,"compose_response":compose})
}
fn body(request: &Request) -> (Value, Value) {
    let b: Value = serde_json::from_slice(&request.body).unwrap();
    let c = serde_json::from_str(b["messages"][0]["content"].as_str().unwrap()).unwrap();
    (b, c)
}
fn response(raw: Value) -> ResponseTemplate {
    crate::claude::test::reply(envelope(raw))
}

#[tokio::test]
async fn slow_optional_prose_returns_verified_receipt_without_repeating_committed_action() {
    let (state, ids) = fixture(&case()).unwrap();
    let goal = ids["Psychology assignment"];
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |request:&Request| {
        let (body,context)=body(request);
        if crate::claude::test::operation(&body)=="buddy_skill_plan" {
            response(action(&context,"complete_step",json!({"goal":{"goal_id":goal,"reference":null},"step":{"step_id":null,"ordinal":1,"reference":null}}),"change",true))
        } else {
            assert_eq!(context["verified_execution"]["status"],"executed");
            response(json!({"message":"Late optional prose","resources":[],"memory_update":null,"verified_result_indices":[0]})).set_delay(Duration::from_secs(30))
        }
    }).expect(2).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let outcome = run_with_deadline(
        &json!({"latest_request":"Mark the first step done","conversation":case().history}),
        conversation,
        &state,
        0,
        true,
        &service,
        // Leave room for planning/execution on busy CI. The independent eight-second
        // optional-response cap must cancel the slow reply, rather than skipping it
        // because the overall deadline expired before the HTTP request even started.
        Instant::now() + Duration::from_secs(60),
    )
    .await
    .unwrap();
    assert!(outcome.committed);
    assert!(!outcome.reply.message.contains("Late optional prose"));
    assert!(outcome.reply.message.contains("classical conditioning"));
    let plan = state.inner.lock().unwrap().storage.goal_plan(goal).unwrap();
    assert!(plan.steps[0].done);
    assert!(!plan.steps[1].done);
}

fn goal_update(target: i64, priority: Value) -> Value {
    json!({"goal":{"goal_id":target,"reference":null},"title":null,"area":null,"description":null,"due_at":null,"priority":priority,"done_when":null,"clear_fields":[]})
}

#[test]
fn verified_search_page_keeps_hidden_goals_addressable_without_widening_workspace() {
    let (state, _) = fixture(&case()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    for n in 0..45 {
        inner
            .storage
            .core_add_goals(
                vec![crate::core::GoalDraft {
                    title: format!("Backlog {n}"),
                    area: "General".into(),
                    steps: vec![],
                }],
                &format!("search-schema-fixture-{n}"),
            )
            .unwrap();
    }
    let conversation = inner.storage.current_conversation().unwrap();
    let before = skills::snapshot(&inner.storage, conversation).unwrap();
    let target = before.data["goals"].as_array().unwrap().last().unwrap()["goal_id"]
        .as_i64()
        .unwrap();
    let title = before.data["goals"].as_array().unwrap().last().unwrap()["title"].clone();
    let workspace = dialogue_context::workspace(&before, true, None);
    assert!(!workspace["goals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["goal_id"] == target));
    let plan = skills::Plan {
        kind: "actions".into(),
        state_version: before.version.clone(),
        actions: vec![skills::Action {
            name: "find_goals".into(),
            instruction: None,
            input: json!({"query":title,"status":null,"area":null,"priority":null,"date":null,"overdue":null,"offset":0,"limit":20}),
        }],
        clarification: None,
    };
    skills::execute(
        &mut inner.storage,
        &before,
        &plan,
        "Find that backlog goal",
        true,
        false,
    )
    .unwrap();
    let after = skills::snapshot(&inner.storage, conversation).unwrap();
    let context = json!({"state":dialogue_context::workspace(&after, true, None)});
    assert_eq!(context["state"]["goals"].as_array().unwrap().len(), 40);
    assert_eq!(
        context["state"]["last_goal_search"]["goal_ids"],
        json!([target])
    );
    let schema = schema(&after, true, &context);
    let read = schema["properties"]["response"]["anyOf"][1]["properties"]["actions"]["items"]
        ["anyOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["properties"]["name"]["const"] == "inspect_goal")
        .unwrap();
    let target_variants = read["properties"]["input"]["properties"]["goal"]["anyOf"]
        .as_array()
        .unwrap();
    assert!(target_variants
        .iter()
        .any(|v| v["properties"]["goal_id"]["enum"]
            .as_array()
            .is_some_and(|ids| ids.contains(&json!(target)))));
    assert_eq!(
        before.data["focused_goal_id"],
        after.data["focused_goal_id"]
    );
}

#[test]
fn planned_creation_uses_its_own_batch_reference_despite_duplicate_existing_titles() {
    let mut c = case();
    c.kind = "ambiguous_goal".into();
    let (state, ids) = fixture(&c).unwrap();
    let inner = state.inner.lock().unwrap();
    let mut original = skills::snapshot(
        &inner.storage,
        inner.storage.current_conversation().unwrap(),
    )
    .unwrap();
    original.references.last_created_goal_id = Some(ids["Psychology assignment"]);
    let plan = skills::Plan {
        kind: "actions".into(),
        state_version: original.version.clone(),
        actions: vec![
            skills::Action {
                name: "create_planned_goal".into(),
                input: json!({"title":"Fresh goal"}),
                instruction: None,
            },
            skills::Action {
                name: "update_goal".into(),
                input: json!({"goal":{"goal_id":null,"reference":"last_created"}}),
                instruction: None,
            },
        ],
        clarification: None,
    };
    assert!(duplicate_target(&plan, &original, None).is_none());
}

#[tokio::test]
async fn enhanced_goal_edit_keeps_semantic_authorization_review_and_canonical_receipt() {
    let (state, ids) = fixture(&case()).unwrap();
    let target = ids["Psychology assignment"];
    let focus = ids["Client reply"];
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |request:&Request|{
        let (b,c)=body(request);
        match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
            "buddy_skill_plan"=>{
                assert!(crate::claude::test::system(&b).as_str().unwrap().contains("create_planned_goal"));
                response(action(&c,"update_goal",goal_update(target,json!("high")),"change",true))
            }
            "buddy_dialogue_authorization"=>response(json!({"review":{"verdict":"approved","request_mode":"change","message":""}})),
            "buddy_dialogue_response"=>{
                assert_eq!(c["verified_execution"]["status"],"executed");
                assert_eq!(c["verified_execution"]["results"][0]["data"]["priority"],"high");
                assert!(c["verified_execution"]["results"][0]["data"].get("checkpoint").is_some());
                assert!(c["canonical_receipt"].as_str().unwrap().contains("Priority: high"));
                response(json!({"message":"Updated Psychology assignment.\n\n- Priority: high","resources":[],"memory_update":null,"verified_result_indices":[0]}))
            }
            "buddy_dialogue_review"=>response(json!({"review":{"verdict":"approved","request_mode":"answer","message":""},"checks":[{"requirement":"Report the verified requested priority change","satisfied":true}]})),
            name=>panic!("Unexpected request {name}")
        }
    }).expect(4).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let result=run(&json!({"latest_request":"Set Psychology assignment priority to high","conversation":case().history,"natural_reply":true}),conversation,&state,0,true,&service).await.unwrap();
    assert!(result.committed);
    assert!(result.reply.message.contains("Priority: high"));
    let core = state
        .inner
        .lock()
        .unwrap()
        .storage
        .core_snapshot(None)
        .unwrap();
    assert_eq!(core.focused_goal_id, Some(focus));
    assert_eq!(
        core.goals
            .iter()
            .find(|g| g.id == target)
            .unwrap()
            .priority
            .as_deref(),
        Some("high")
    );
}

#[tokio::test]
async fn dialogue_can_read_then_undo_using_original_scoped_journal() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let (state, ids) = fixture(&case()).unwrap();
    let target = ids["Psychology assignment"];
    let conversation;
    {
        let mut inner = state.inner.lock().unwrap();
        conversation = inner.storage.current_conversation().unwrap();
        let snap = skills::snapshot(&inner.storage, conversation).unwrap();
        let plan = skills::Plan {
            kind: "actions".into(),
            state_version: snap.version.clone(),
            actions: vec![skills::Action {
                name: "update_goal".into(),
                input: goal_update(target, json!("high")),
                instruction: Some("Set priority high".into()),
            }],
            clarification: None,
        };
        skills::execute(
            &mut inner.storage,
            &snap,
            &plan,
            "Set priority high",
            true,
            false,
        )
        .unwrap();
    }
    let server = MockServer::start().await;
    let rounds = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST")).respond_with(move |request:&Request|{
        let (b,c)=body(request);
        match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
            "buddy_skill_plan" if rounds.fetch_add(1,Ordering::SeqCst)==0=>response(action(&c,"inspect_goal",json!({"goal":{"goal_id":target,"reference":null}}),"change",true)),
            "buddy_skill_plan"=>{
                assert_eq!(c["state"]["references"]["last_action"]["skill"],"inspect_goal");
                assert_eq!(c["state"]["last_goal_edit"]["goal_id"],target);
                response(action(&c,"undo_last_goal_edit",json!({}),"change",true))
            }
            "buddy_dialogue_authorization"=>{
                assert_eq!(c["original_user_context"]["last_goal_edit"]["goal_id"],target);
                response(json!({"review":{"verdict":"approved","request_mode":"change","message":""}}))
            }
            "buddy_dialogue_response"=>response(json!({"message":"Undid the previous priority edit.","resources":[],"memory_update":null,"verified_result_indices":[0]})),
            "buddy_dialogue_review"=>response(json!({"review":{"verdict":"approved","request_mode":"answer","message":""},"checks":[{"requirement":"Report only the verified undo","satisfied":true}]})),
            name=>panic!("Unexpected request {name}")
        }
    }).expect(5).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let result=run(&json!({"latest_request":"Undo my previous priority edit","conversation":[],"natural_reply":true}),conversation,&state,0,true,&service).await.unwrap();
    assert!(result.committed);
    let core = state
        .inner
        .lock()
        .unwrap()
        .storage
        .core_snapshot(None)
        .unwrap();
    assert!(core
        .goals
        .iter()
        .find(|g| g.id == target)
        .unwrap()
        .priority
        .is_none());
}

#[test]
fn semantic_validation_rejects_mutation_in_read_intent_and_wrong_operation() {
    let c = json!({"state":{"state_version":"v"},"latest_request":"Read only"});
    let mut raw = action(
        &c,
        "delete_goal",
        json!({"goal":{"goal_id":1,"reference":null}}),
        "read",
        false,
    );
    assert!(parse(&envelope(raw.clone()), "Read only").is_err());
    raw["intent"] = json!({"mode":"change","operations":["rename_goal"]});
    assert!(parse(&envelope(raw), "Rename it").is_err());
    for bad in [
        json!(null),
        json!(true),
        json!([]),
        json!("not an object"),
        json!({"intent":null}),
    ] {
        assert!(parse(&envelope(bad), "Hello").is_err());
    }
}

#[tokio::test]
async fn conversational_answer_sees_queue_and_uses_one_request_for_text_and_voice() {
    for event in ["user_message", "voice_message"] {
        let (state, ids) = fixture(&case()).unwrap();
        let conversation = state
            .inner
            .lock()
            .unwrap()
            .storage
            .current_conversation()
            .unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |r: &Request| {
                let (b, c) = body(r);
                assert_eq!(
                    b["messages"].as_array().unwrap().last().unwrap()["content"],
                    "It is in Up next"
                );
                assert_eq!(b["messages"][1]["role"], "user");
                assert_eq!(
                    b["messages"][1]["content"],
                    "Help me with Psychology assignment"
                );
                assert!(c.get("conversation").is_none());
                assert_eq!(
                    c["state"]["work_queue"]["in_focus"]["title"],
                    "Client reply"
                );
                assert_eq!(
                    c["state"]["work_queue"]["up_next_goal_id"],
                    ids["Psychology assignment"]
                );
                assert!(c["product_facts"].as_str().unwrap().contains("Up next"));
                assert!(c["state"]["goals"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|g| g["title"] == "Psychology assignment"
                        && g["plan"]["steps"].as_array().unwrap().len() == 3));
                response(answer(
                    &c,
                    "Start by explaining classical conditioning using the saved assignment.",
                ))
            })
            .expect(1)
            .mount(&server)
            .await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let reply=agent_runtime::handle_dialogue(&json!({"latest_request":"It is in Up next","conversation":case().history,"event_type":event}),conversation,&state,0,true,&service).await.unwrap();
        assert!(reply.reply.message.contains("classical conditioning"));
        assert!(!reply.committed);
        assert_eq!(reply.revision, 0);
    }
}

#[tokio::test]
async fn read_results_update_reference_then_answer_without_redundant_classifier() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let (state, ids) = fixture(&case()).unwrap();
    let psyche = ids["Psychology assignment"];
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST")).respond_with(move |r:&Request|{let (b,ctx)=body(r);assert_eq!(crate::claude::test::operation(&b),"buddy_skill_plan");
        if count.fetch_add(1,Ordering::SeqCst)==0 {response(action(&ctx,"inspect_goal",json!({"goal":{"goal_id":psyche,"reference":null}}),"read",true))}
        else {assert_eq!(ctx["tool_results"][0]["results"][0]["goal_id"],psyche);
            assert_eq!(ctx["state"]["references"]["last_action"]["skill"],"inspect_goal");
            response(answer(&ctx,"The first step is to explain classical conditioning. Give an example after that."))}
    }).expect(2).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out = run(
        &json!({"latest_request":"What remains there?","conversation":case().history}),
        c,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap();
    assert!(!out.committed);
    assert!(out.reply.message.contains("classical conditioning"));
    let inner = state.inner.lock().unwrap();
    assert_eq!(
        inner
            .storage
            .action_references(c)
            .unwrap()
            .last_referenced_goal_id,
        Some(psyche)
    );
    assert_eq!(
        inner.storage.core_snapshot(None).unwrap().focused_goal_id,
        Some(ids["Client reply"])
    );
}

#[tokio::test]
async fn mutation_composition_receives_verified_result_only_after_commit() {
    let (state, ids) = fixture(&case()).unwrap();
    let psyche = ids["Psychology assignment"];
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |r:&Request|{let (b,ctx)=body(r);
        if crate::claude::test::operation(&b)=="buddy_skill_plan" {response(action(&ctx,"complete_step",json!({"goal":{"goal_id":psyche,"reference":null},"step":{"step_id":null,"ordinal":1,"reference":null}}),"change",true))}
        else {assert_eq!(ctx["verified_execution"]["status"],"executed");assert_eq!(ctx["verified_execution"]["results"][0]["data"]["steps"][0]["done"],true);
            response(json!({"message":"The first step is complete. Next, give an everyday example.","resources":[],"memory_update":null,"verified_result_indices":[0]}))}
    }).expect(2).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out = run(
        &json!({"latest_request":"Mark the first step done","conversation":case().history}),
        c,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap();
    assert!(out.committed);
    assert_eq!(out.revision, 1);
    assert!(out.reply.message.contains("everyday example"));
    assert!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .goal_plan(psyche)
            .unwrap()
            .steps[0]
            .done
    );
}

#[tokio::test]
async fn read_intent_cannot_escalate_to_delete_after_inspection() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let (state, ids) = fixture(&case()).unwrap();
    let psyche = ids["Psychology assignment"];
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .respond_with(move |r: &Request| {
            let (_, ctx) = body(r);
            response(if count.fetch_add(1, Ordering::SeqCst) == 0 {
                action(
                    &ctx,
                    "inspect_goal",
                    json!({"goal":{"goal_id":psyche,"reference":null}}),
                    "read",
                    true,
                )
            } else {
                action(
                    &ctx,
                    "delete_goal",
                    json!({"goal":{"goal_id":psyche,"reference":null}}),
                    "change",
                    false,
                )
            })
        })
        .expect(2)
        .mount(&server)
        .await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    assert!(run(
        &json!({"latest_request":"Read the goal"}),
        c,
        &state,
        0,
        true,
        &service
    )
    .await
    .err()
    .unwrap()
    .contains("escalate"));
    assert!(state
        .inner
        .lock()
        .unwrap()
        .storage
        .goal_plan(psyche)
        .is_ok());
}

#[tokio::test]
async fn sharing_off_hides_workspace_and_preferences_before_any_provider_call() {
    let (state, ids) = fixture(&case()).unwrap();
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| {
            let (_, ctx) = body(r);
            let serialized = ctx.to_string();
            assert!(!serialized.contains("Psychology assignment"));
            assert!(!serialized.contains("PRIVATE_MEMORY_SENTINEL"));
            assert_eq!(ctx["state"]["sharing"], false);
            assert!(ctx["state"]["work_queue"].is_null());
            response(answer(
                &ctx,
                "Goal sharing is off, so I cannot read your saved goals.",
            ))
        })
        .expect(1)
        .mount(&server)
        .await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out=run(&json!({"latest_request":"What is next?","selected_goal_id":ids["Psychology assignment"],"personal_memory":{"secret":"PRIVATE_MEMORY_SENTINEL"}}),c,&state,0,false,&service).await.unwrap();
    assert!(!out.committed);
}

#[test]
fn shared_context_limits_details_and_preserves_saved_queue_order() {
    let (state, ids) = fixture(&case()).unwrap();
    let inner = state.inner.lock().unwrap();
    let snap = skills::snapshot(
        &inner.storage,
        inner.storage.current_conversation().unwrap(),
    )
    .unwrap();
    let workspace = dialogue_context::workspace(&snap, true, None);
    assert_eq!(
        workspace["work_queue"]["later_today"][0]["goal_id"],
        ids["Psychology assignment"]
    );
    assert!(workspace["goals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["title"] == "Physics reading")
        .unwrap()["plan"]
        .is_null());
    assert!(workspace.to_string().len() < 24000);
    assert!(dialogue_context::workspace(&snap, false, None)["goals"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn evaluation_contains_sixty_cases_in_three_languages_without_production_data() {
    let cases = crate::dialogue_benchmark::cases().unwrap();
    assert_eq!(cases.len(), 60);
    for language in ["ru", "en", "he"] {
        assert_eq!(cases.iter().filter(|c| c.language == language).count(), 20);
    }
}

#[tokio::test]
async fn semantic_preflight_blocks_guessed_target_without_writes() {
    let mut scenario = case();
    scenario.kind = "ambiguous_pronoun".into();
    scenario.history.clear();
    let (state, ids) = fixture(&scenario).unwrap();
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let target = ids["Client reply"];
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |r:&Request| {
        let (b,ctx)=body(r);
        if crate::claude::test::operation(&b)=="buddy_skill_plan" {
            let mut plan=action(&ctx,"delete_goal",json!({"goal":{"goal_id":target,"reference":null}}),"change",false);
            plan.as_object_mut().unwrap().remove("intent"); plan["decision"]=json!("change"); response(plan)
        } else {
            assert_eq!(crate::claude::test::operation(&b),"buddy_dialogue_authorization");
            assert!(ctx["original_user_context"]["references"]["last_referenced_goal_id"].is_null());
            response(json!({"verdict":"clarify","request_mode":"clarification","message":"Which goal should I delete?"}))
        }
    }).expect(2).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out = run(
        &json!({"latest_request":"Delete it","natural_reply":true}),
        c,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap();
    assert!(!out.committed);
    assert!(out.reply.message.contains("Which goal"));
    let i = state.inner.lock().unwrap();
    assert_eq!(i.storage.core_snapshot(None).unwrap().goals.len(), 5);
    assert!(i
        .storage
        .action_references(c)
        .unwrap()
        .last_action
        .is_none());
}

#[tokio::test]
async fn unexecuted_mutation_claim_is_repaired_before_display() {
    let (state, _) = fixture(&case()).unwrap();
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|r:&Request| {
        let (b,ctx)=body(r);
        match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
            "buddy_skill_plan"=>response(answer(&ctx,"I switched your focus to Psychology assignment.")),
            "buddy_dialogue_review" if ctx["proposal"]["message"].as_str().unwrap().contains("switched") => {
                assert!(ctx["proposal"]["verified_execution"].is_null());
                response(json!({"verdict":"repair","request_mode":"answer","message":"No focus change occurred. Explain the saved next step instead."}))
            },
            "buddy_dialogue_response"=>{
                assert!(ctx["reply_correction"].as_str().unwrap().contains("No focus change"));
                response(json!({"message":"Psychology assignment is Up next. Start with classical conditioning.","resources":[],"memory_update":null,"verified_result_indices":[]}))
            },
            "buddy_dialogue_review"=>response(json!({"verdict":"approved","request_mode":"answer","message":""})),
            other=>panic!("Unexpected operation {other}"),
        }
    }).expect(4).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out=run(&json!({"latest_request":"It is in Up next","natural_reply":true,"conversation":case().history}),c,&state,0,true,&service).await.unwrap();
    assert!(!out.committed);
    assert!(!out.reply.message.contains("switched"));
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .focused_goal_id,
        Some(1)
    );
}

#[tokio::test]
async fn unsupported_pilot_claim_is_repaired_without_mutating_saved_steps() {
    let (state, _) = fixture(&case()).unwrap();
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|r:&Request| {
        let (b,c)=body(r);
        let system=crate::claude::test::system(&b).as_str().unwrap();
        match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
            "buddy_skill_plan"=>response(answer(&c,"The pilot proved a 15% revenue increase.")),
            "buddy_dialogue_review" if c["proposal"]["message"].as_str().unwrap().contains("15%")=>{
                assert!(system.contains("pilot results"));
                assert!(system.contains("Earlier assistant drafts"));
                response(json!({"verdict":"repair","request_mode":"answer","message":"There are no pilot results. Rewrite only slide two as a proposed pilot within the stated budget."}))
            },
            "buddy_dialogue_response"=>{
                assert!(system.contains("Never invent product capabilities"));
                assert!(c["reply_correction"].as_str().unwrap().contains("no pilot results"));
                response(json!({"message":"Pilot proposal: test the order queue within the 18000-ruble budget; results are not yet known.","resources":[],"memory_update":null,"verified_result_indices":[]}))
            },
            "buddy_dialogue_review"=>response(json!({"verdict":"approved","request_mode":"answer","message":""})),
            name=>panic!("Unexpected operation {name}"),
        }
    }).expect(4).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let before = json!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .goals
    );
    let result=run(&json!({"latest_request":"Rewrite only the second slide. Budget 18000. No invented pilot results or saved step changes.","conversation":[],"natural_reply":true}),conversation,&state,0,true,&service).await.unwrap();
    assert!(!result.committed);
    assert!(result.reply.message.contains("results are not yet known"));
    assert!(!result.reply.message.contains("15%"));
    assert_eq!(
        json!(
            state
                .inner
                .lock()
                .unwrap()
                .storage
                .core_snapshot(None)
                .unwrap()
                .goals
        ),
        before
    );
}

#[tokio::test]
async fn dialogue_recovers_older_chat_fact_through_read_skill_with_goal_sharing_off() {
    let (state, _) = fixture(&case()).unwrap();
    let conversation = {
        let mut inner = state.inner.lock().unwrap();
        let conversation = inner.storage.current_conversation().unwrap();
        inner
            .storage
            .save_chat_turn(
                "Laguna reference ORBIT-472",
                &ChatReply {
                    message: "Understood".into(),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        inner.storage.create_conversation().unwrap();
        inner
            .storage
            .save_chat_turn(
                "Other chat reference WRONG-999",
                &ChatReply {
                    message: "Understood".into(),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        conversation
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|r: &Request| {
            let (b, c) = body(r);
            let name = crate::claude::test::operation(&b).as_str().unwrap();
            assert!(c["state"]["goals"].as_array().unwrap().is_empty());
            assert!(!c.to_string().contains("WRONG-999"));
            if name == "buddy_skill_plan" && c["tool_results"].as_array().unwrap().is_empty() {
                response(action(
                    &c,
                    "inspect_conversation",
                    json!({"query":"LAGUNA","before_message_id":null}),
                    "read",
                    false,
                ))
            } else if name == "buddy_skill_plan" {
                assert!(c["tool_results"].to_string().contains("ORBIT-472"));
                response(answer(&c, "ORBIT-472"))
            } else {
                assert_eq!(name, "buddy_dialogue_review");
                response(json!({"verdict":"approved","request_mode":"read","message":""}))
            }
        })
        .expect(3)
        .mount(&server)
        .await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let result=run(&json!({"latest_request":"Read our old Laguna reference from this chat","conversation":[],"natural_reply":true}),conversation,&state,0,false,&service).await.unwrap();
    assert_eq!(result.reply.message, "ORBIT-472");
    assert!(!result.committed);
}

#[tokio::test]
async fn authorization_outage_fails_closed_before_mutation() {
    let (state, ids) = fixture(&case()).unwrap();
    let target = ids["Psychology assignment"];
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(move |r: &Request| {
            let (b, ctx) = body(r);
            if crate::claude::test::operation(&b) == "buddy_skill_plan" {
                response(action(
                    &ctx,
                    "set_focus_goal",
                    json!({"goal":{"goal_id":target,"reference":null}}),
                    "change",
                    false,
                ))
            } else {
                ResponseTemplate::new(503)
            }
        })
        .mount(&server)
        .await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    assert!(run(
        &json!({"latest_request":"Focus Psychology assignment","natural_reply":true}),
        c,
        &state,
        0,
        true,
        &service
    )
    .await
    .is_err());
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .focused_goal_id,
        Some(ids["Client reply"])
    );
}

#[test]
fn context_budget_omissions_are_explicit_and_latest_request_is_preserved() {
    let (state, _) = fixture(&case()).unwrap();
    let i = state.inner.lock().unwrap();
    let snapshot = skills::snapshot(&i.storage, i.storage.current_conversation().unwrap()).unwrap();
    let history: Vec<_> = (0..12)
        .map(|_| json!({"role":"user","text":"Ж".repeat(2000)}))
        .collect();
    let packet = dialogue_context::packet(
        &json!({"latest_request":"Read this goal","conversation":history,"attachment_text":"a".repeat(50000)}),
        &snapshot,
        true,
        &[],
    );
    assert_eq!(packet["latest_request"], "Read this goal");
    assert!(
        packet["context_budget"]["omitted_older_messages"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(packet.to_string().len() <= 96 * 1024);
}

#[test]
fn natural_prose_keeps_complete_multiline_answers_and_cannot_execute_json_drafts() {
    let text = "1. Problem\nDescribe lost orders.\n\n2. Proposed solution\nTest an order queue.\n\n3. Budget\nStay within 18000 rubles.";
    let value = json!({"choices":[{"finish_reason":"stop","message":{"content":text}}]});
    let reply = prose_reply(&value, &[], &[]).unwrap();
    assert_eq!(reply.message, text);
    assert!(reply.resources.is_empty());
    let draft = "{\"name\":\"create_goal\",\"input\":{\"title\":\"Example JSON\"}}";
    let value = json!({"choices":[{"finish_reason":"stop","message":{"content":draft}}]});
    assert_eq!(prose_reply(&value, &[], &[]).unwrap().message, draft);
    let draft = "{\"message\":\"A user-requested JSON example\"}";
    let value = json!({"choices":[{"finish_reason":"stop","message":{"content":draft}}]});
    assert_eq!(prose_reply(&value, &[], &[]).unwrap().message, draft);
    let bad = json!({"choices":[{"finish_reason":"stop","message":{"content":"Read https://invented.example/results"}}]});
    assert!(prose_reply(&bad, &[], &[]).is_err());
    let incomplete = json!({"choices":[{"finish_reason":"length","message":{"content":text}}]});
    assert!(prose_reply(&incomplete, &[], &[]).is_err());
}

#[test]
fn response_review_repairs_unmet_requirements_and_accepts_contextual_paraphrases() {
    let mut review = Review {
        verdict: "approved".into(),
        request_mode: "answer".into(),
        message: String::new(),
    };
    let value = |checks: Value| {
        envelope(
            json!({"checks":checks,"review":{"verdict":"approved","request_mode":"answer","message":""}}),
        )
    };
    validate_review_checks(&value(json!([{"requirement":"Return only the edited second slide; omit unchanged slides", "satisfied":false}])),&mut review).unwrap();
    assert_eq!(review.verdict, "repair");
    assert!(review.message.contains("omit unchanged slides"));
    assert!(review.message.chars().count() <= 500);
    let mut approved = Review {
        verdict: "approved".into(),
        request_mode: "answer".into(),
        message: String::new(),
    };
    validate_review_checks(&value(json!([{"requirement":"Recall the corrected budget from the preceding conversation", "satisfied":true}])),&mut approved).unwrap();
    assert_eq!(approved.verdict, "approved");
    assert!(validate_review_checks(&value(json!([])), &mut approved).is_err());
    assert!(validate_review_checks(
        &value(json!([{"requirement":"", "satisfied":true}])),
        &mut approved
    )
    .is_err());
}

#[tokio::test]
async fn writer_uses_plain_text_while_actions_and_review_keep_structured_json() {
    let (state, _) = fixture(&case()).unwrap();
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let snapshot = fresh(&state, conversation, 0).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|r:&Request| {
        let (b,_) = body(r);
        assert!(b["output_config"].get("format").is_none());
        assert!(crate::claude::test::system(&b).as_str().unwrap().contains("without a JSON envelope"));
        crate::claude::test::reply(json!({"model":"fixture","choices":[{"finish_reason":"stop","message":{"content":"Slide 1: Problem\nSlide 2: Proposed pilot\nSlide 3: Budget"}}],"usage":{"prompt_tokens":10,"completion_tokens":20}}))
    }).expect(1).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let reply = present(
        &state,
        &service,
        &json!({"latest_request":"Write three slides", "conversation":[]}),
        &snapshot,
        true,
        &[],
        None,
        &[],
        0,
    )
    .await
    .unwrap();
    assert_eq!(reply.message.lines().count(), 3);
    let inner = state.inner.lock().unwrap();
    let view = crate::engine_observability::view(&inner).unwrap();
    assert_eq!(view["last_inference"]["provider_reported_model"], "fixture");
    assert_eq!(view["last_inference"]["completion_tokens"], 20);
    assert!(skills::plan_schema()["properties"]["actions"].is_object());
}

#[tokio::test]
async fn verbal_delete_confirmation_is_repaired_into_backend_pending_preview() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let (state, ids) = fixture(&case()).unwrap();
    let target = ids["Psychology assignment"];
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    let plans = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST")).respond_with(move |r:&Request|{
        let (b,c)=body(r);
        match crate::claude::test::operation(&b).as_str().unwrap() {
            "buddy_skill_plan" if plans.fetch_add(1,Ordering::SeqCst)==0 => response(json!({"state_version":c["state"]["state_version"],"response":{"decision":"clarification","question":"Are you sure you want to delete the second step?"}})),
            "buddy_dialogue_review" => {
                assert!(crate::claude::test::system(&b).as_str().unwrap().contains("backend can create its preview"));
                response(json!({"review":{"verdict":"repair","request_mode":"change","message":"The step is unambiguous. Plan delete_step so the backend creates a confirmation preview, not a verbal question."}}))
            },
            "buddy_skill_plan" => response(action(&c,"delete_step",json!({"goal":{"goal_id":target,"reference":null},"step":{"step_id":null,"ordinal":2,"reference":null}}),"change",false)),
            "buddy_dialogue_authorization" => response(json!({"review":{"verdict":"approved","request_mode":"change","message":""}})),
            operation => panic!("Unexpected operation {operation}"),
        }
    }).expect(4).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let result=run(&json!({"latest_request":"Delete the second step of Psychology assignment", "natural_reply":true,"conversation":[]}),conversation,&state,0,true,&service).await.unwrap();
    assert!(!result.committed);
    let inner = state.inner.lock().unwrap();
    assert_eq!(inner.storage.goal_plan(target).unwrap().steps.len(), 3);
    assert!(skills::pending_confirmation(&inner.storage, conversation)
        .unwrap()
        .is_some());
}

#[test]
fn conversation_context_preserves_earlier_constraints_corrections_and_full_recent_drafts() {
    let (state, _) = fixture(&case()).unwrap();
    let inner = state.inner.lock().unwrap();
    let snap = skills::snapshot(
        &inner.storage,
        inner.storage.current_conversation().unwrap(),
    )
    .unwrap();
    let mut history =
        vec![json!({"id":1,"role":"user","text":"Mayak: 5 minutes, cafe owners, budget 30000"})];
    history.extend(
        (2..=25).map(|id| json!({"id":id,"role":"assistant","text":"A short synthetic exchange"})),
    );
    history.push(json!({"id":26,"role":"user","text":"Correction: budget is 18000"}));
    let draft = format!("{} THE_FINAL_SENTENCE", "a".repeat(5500));
    history.push(json!({"id":27,"role":"assistant","text":draft}));
    let packet = dialogue_context::packet(
        &json!({"latest_request":"Shorten the second section", "conversation":history}),
        &snap,
        false,
        &[],
    );
    assert_eq!(packet["conversation"].as_array().unwrap().len(), 27);
    assert!(packet["conversation"].to_string().contains("cafe owners"));
    assert!(packet["conversation"]
        .to_string()
        .contains("Correction: budget is 18000"));
    assert!(packet["conversation"]
        .to_string()
        .contains("THE_FINAL_SENTENCE"));
    assert_eq!(packet["context_budget"]["omitted_older_messages"], 0);
    assert_eq!(packet["context_budget"]["clipped_messages"], 0);
    assert!(packet["state"]["goals"].as_array().unwrap().is_empty());
}

#[test]
fn history_clipping_and_count_limits_are_visible_and_utf8_safe() {
    let (state, _) = fixture(&case()).unwrap();
    let inner = state.inner.lock().unwrap();
    let snap = skills::snapshot(
        &inner.storage,
        inner.storage.current_conversation().unwrap(),
    )
    .unwrap();
    let mut history: Vec<_> = (1..=119)
        .map(|id| json!({"id":id,"role":"user","text":"short"}))
        .collect();
    history.push(json!({"id":120,"role":"assistant","text":"🙂".repeat(7000)}));
    let packet = dialogue_context::packet(
        &json!({"latest_request":"Continue", "conversation":history}),
        &snap,
        true,
        &[],
    );
    assert_eq!(packet["context_budget"]["omitted_older_messages"], 20);
    assert_eq!(packet["context_budget"]["clipped_messages"], 1);
    assert_eq!(
        packet["conversation"].as_array().unwrap().last().unwrap()["text"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        6000
    );
    assert_eq!(
        packet["conversation"].as_array().unwrap().last().unwrap()["clipped"],
        true
    );
    assert!(packet.to_string().len() <= 96 * 1024);
}

#[tokio::test]
async fn confirmation_in_another_language_executes_only_saved_proposal() {
    let (state, ids) = fixture(&case()).unwrap();
    let target = ids["Psychology assignment"];
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    {
        let mut i = state.inner.lock().unwrap();
        let mut snap = skills::snapshot(&i.storage, c).unwrap();
        snap.privacy_revision = Some(0);
        let plan:skills::Plan=serde_json::from_value(json!({"kind":"actions","state_version":snap.version,"actions":[{"name":"delete_goal","input":{"goal":{"goal_id":target,"reference":null}},"instruction":"Delete Psychology assignment"}],"clarification":null})).unwrap();
        let execution = skills::execute_for_event(
            &mut i.storage,
            &snap,
            &plan,
            "Delete Psychology assignment",
            true,
            false,
            agent_runtime::EventKind::VoiceMessage,
        )
        .unwrap();
        assert_eq!(execution.status, "confirmation_required");
    }
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |r:&Request| {
        let (b,ctx)=body(r);
        match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
            "buddy_dialogue_confirmation"=>{
                assert_eq!(ctx["pending_proposal"]["actions"][0]["input"]["goal"]["goal_id"],target);
                response(json!({"decision":"confirm"}))
            },
            "buddy_dialogue_response"=>response(json!({"message":"היעד נמחק. אפשר לשחזר אותו.","resources":[],"memory_update":null,"verified_result_indices":[0]})),
            "buddy_dialogue_review"=>response(json!({"verdict":"approved","request_mode":"change","message":""})),
            other=>panic!("Unexpected operation {other}"),
        }
    }).expect(3).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out = run(
        &json!({"latest_request":"כן, אני מאשר למחוק את היעד הזה","natural_reply":true}),
        c,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap();
    assert!(out.committed);
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .goals
            .len(),
        4
    );
}

#[tokio::test]
async fn privacy_change_during_authorization_blocks_execution() {
    let (state, ids) = fixture(&case()).unwrap();
    let target = ids["Psychology assignment"];
    let state = std::sync::Arc::new(state);
    let captured = state.clone();
    let c = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(move |r: &Request| {
            let (b, ctx) = body(r);
            if crate::claude::test::operation(&b) == "buddy_skill_plan" {
                response(action(
                    &ctx,
                    "set_focus_goal",
                    json!({"goal":{"goal_id":target,"reference":null}}),
                    "change",
                    false,
                ))
            } else {
                captured.inner.lock().unwrap().privacy_revision += 1;
                response(json!({"verdict":"approved","request_mode":"change","message":""}))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    assert!(run(
        &json!({"latest_request":"Focus Psychology assignment","natural_reply":true}),
        c,
        &state,
        0,
        true,
        &service
    )
    .await
    .is_err());
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .focused_goal_id,
        Some(ids["Client reply"])
    );
}

#[test]
fn exclusive_response_schema_removes_conflicting_decisions_and_memory_fields() {
    let (state, _) = fixture(&case()).unwrap();
    let i = state.inner.lock().unwrap();
    let snapshot = skills::snapshot(&i.storage, i.storage.current_conversation().unwrap()).unwrap();
    let context = dialogue_context::packet(
        &json!({"latest_request":"What remains?"}),
        &snapshot,
        true,
        &[],
    );
    let format = schema(&snapshot, true, &context);
    let variants = format["properties"]["response"]["anyOf"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 5);
    let read = &variants[1];
    assert_eq!(read["properties"]["decision"]["const"], "read");
    assert!(read["properties"].get("updates").is_none());
    assert!(!read["properties"]["actions"]["items"]["anyOf"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["properties"]["name"]["const"] == "delete_goal"));
    let raw = json!({"state_version":snapshot.version,"response":{"decision":"answer","message":"Three saved steps remain.","web_query":null,"compose_response":false}});
    assert_eq!(
        parse(&envelope(raw), "What remains?").unwrap().intent.mode,
        "answer"
    );
    let invalid = json!({"state_version":snapshot.version,"response":{"decision":"answer","message":"Done","web_query":null,"compose_response":false,"actions":[]}});
    assert!(parse(&envelope(invalid), "What remains?").is_err());
    let invalid = json!({"state_version":snapshot.version,"response":{"decision":"read","actions":[],"compose_response":true}});
    assert!(parse(&envelope(invalid), "What remains?").is_err());
}

#[test]
fn punctuation_only_provider_responses_are_not_displayable_answers() {
    let raw = json!({"state_version":"v","response":{"decision":"answer","message":"...","web_query":null,"compose_response":false}});
    assert!(parse(&envelope(raw), "Help me").is_err());
    assert!(crate::ai_chat::parse_chat(
        &envelope(json!({"message":"...","resources":[],"memory_update":null})),
        &[]
    )
    .is_err());
}

#[test]
fn ordered_schema_selects_decision_and_skill_before_arguments() {
    let (state, _) = fixture(&case()).unwrap();
    let i = state.inner.lock().unwrap();
    let snap = skills::snapshot(&i.storage, i.storage.current_conversation().unwrap()).unwrap();
    let context = dialogue_context::packet(
        &json!({"latest_request":"Focus the goal"}),
        &snap,
        true,
        &[],
    );
    let format = schema(&snap, true, &context);
    let change = &format["properties"]["response"]["anyOf"][2];
    assert_eq!(
        change["properties"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap(),
        "decision"
    );
    let action = &change["properties"]["actions"]["items"]["anyOf"][0];
    assert_eq!(
        action["properties"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap(),
        "name"
    );
    let approved = json!({"review":{"verdict":"approved","request_mode":"change","message":""}});
    assert!(parse_review(&envelope(approved), true).is_ok());
    let bad = json!({"review":{"verdict":"approved","request_mode":"answer","message":"I changed your focus."}});
    assert!(parse_review(&envelope(bad), false).is_err());
    let mut a = json!({"name":"inspect_goal","input":{"goal":{"goal_id":1,"reference":null}}});
    let mut b: Value = serde_json::from_str(
        r#"{"input":{"goal":{"reference":null,"goal_id":1}},"name":"inspect_goal"}"#,
    )
    .unwrap();
    a.sort_all_objects();
    b.sort_all_objects();
    assert_eq!(a.to_string(), b.to_string());
}

#[tokio::test]
async fn recognized_action_replans_instead_of_answering_or_asking_again() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    for initial_clarification in [true, false] {
        let (state, ids) = fixture(&case()).unwrap();
        let goal = ids["Psychology assignment"];
        let c = state
            .inner
            .lock()
            .unwrap()
            .storage
            .current_conversation()
            .unwrap();
        let server = MockServer::start().await;
        let plans = Arc::new(AtomicUsize::new(0));
        let reviews = Arc::new(AtomicUsize::new(0));
        Mock::given(method("POST")).respond_with(move |r:&Request| {
            let (b,ctx)=body(r);
            match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
                "buddy_skill_plan" if plans.fetch_add(1,Ordering::SeqCst)==0=>{
                    response(json!({"state_version":ctx["state"]["state_version"],"response":if initial_clarification {
                        json!({"decision":"clarification","question":"Do you want to focus Psychology assignment?"})
                    }else{json!({"decision":"answer","message":"Focus switched to Psychology assignment.","web_query":null,"compose_response":false})}}))
                },
                "buddy_skill_plan"=>{
                    assert_eq!(ctx["validation_feedback"]["request_mode"],"change");
                    response(action(&ctx,"set_focus_goal",json!({"goal":{"goal_id":goal,"reference":null}}),"change",true))
                },
                "buddy_dialogue_authorization"=>response(json!({"review":{"verdict":"approved","request_mode":"change","message":""}})),
                "buddy_dialogue_response"=>{
                    assert_eq!(ctx["verified_execution"]["status"],"executed");
                    response(json!({"message":"Psychology assignment is now in focus.","resources":[],"memory_update":null,"verified_result_indices":[0]}))
                },
                "buddy_dialogue_review" if reviews.fetch_add(1,Ordering::SeqCst)==0=>response(json!({"review":{"verdict":if initial_clarification {"repair"}else{"approved"},"request_mode":"change","message":if initial_clarification {"The user explicitly requested set_focus_goal. Execute it instead of asking again."}else{""}}})),
                "buddy_dialogue_review"=>response(json!({"review":{"verdict":"approved","request_mode":"change","message":""}})),
                name=>panic!("Unexpected request {name}"),
            }
        }).expect(6).mount(&server).await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let out=run(&json!({"latest_request":"Focus Psychology assignment","natural_reply":true,"conversation":case().history}),c,&state,0,true,&service).await.unwrap();
        assert!(out.committed);
        assert_eq!(out.revision, 1);
        assert_eq!(
            state
                .inner
                .lock()
                .unwrap()
                .storage
                .core_snapshot(None)
                .unwrap()
                .focused_goal_id,
            Some(goal)
        );
    }
}

#[test]
fn duplicate_names_require_a_stable_selection_even_if_model_guesses() {
    let mut c = case();
    c.kind = "ambiguous_goal".into();
    let (state, ids) = fixture(&c).unwrap();
    let i = state.inner.lock().unwrap();
    let snap = skills::snapshot(&i.storage, i.storage.current_conversation().unwrap()).unwrap();
    let goal = ids["Psychology assignment"];
    let plan:skills::Plan=serde_json::from_value(json!({"kind":"actions","state_version":snap.version,"actions":[{"name":"set_focus_goal","input":{"goal":{"goal_id":goal,"reference":null}},"instruction":"Focus Psychology assignment"}],"clarification":null})).unwrap();
    let candidates = duplicate_target(&plan, &snap, None).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[1]["step_count"], 1);
    assert!(duplicate_target(&plan, &snap, Some(goal)).is_none());
    let mut referenced = snap.clone();
    referenced.references.last_referenced_goal_id = Some(goal);
    assert!(duplicate_target(&plan, &referenced, None).is_none());
}

#[tokio::test]
async fn duplicate_mutation_is_blocked_before_authorization_or_execution() {
    let mut c = case();
    c.kind = "ambiguous_goal".into();
    let (state, ids) = fixture(&c).unwrap();
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let before = skills::snapshot(&state.inner.lock().unwrap().storage, conversation).unwrap();
    let goal = ids["Psychology assignment"];
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |r: &Request| {
        let (b, ctx) = body(r);
        match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
            "buddy_skill_plan" => response(action(&ctx,"set_focus_goal",json!({"goal":{"goal_id":goal,"reference":null}}),"change",true)),
            "buddy_target_clarification" => response(json!({"question":"Two goals share this title. Which do you mean? Select it in the chat goal selector."})),
            name => panic!("Ambiguous mutation must never reach {name}"),
        }
    }).expect(2).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out = run(&json!({"latest_request":"Focus Psychology assignment","natural_reply":true,"conversation":c.history}),conversation,&state,0,true,&service).await.unwrap();
    assert!(!out.committed);
    assert!(out.reply.message.contains("Two goals"));
    let after = skills::snapshot(&state.inner.lock().unwrap().storage, conversation).unwrap();
    assert_eq!(before.data, after.data);
    assert_eq!(
        before.data["focused_goal_id"],
        after.data["focused_goal_id"]
    );
    assert_eq!(
        before.references.last_referenced_goal_id,
        after.references.last_referenced_goal_id
    );
}

#[tokio::test]
async fn mixed_version_configuration_path_is_repaired_before_review_can_approve_it() {
    let (state, _) = fixture(&case()).unwrap();
    let snapshot = {
        let inner = state.inner.lock().unwrap();
        skills::snapshot(
            &inner.storage,
            inner.storage.current_conversation().unwrap(),
        )
        .unwrap()
    };
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|r:&Request| {
        let (body,context)=body(r);
        match crate::claude::test::operation(&body).as_str() {
            None=> {
                assert!(context["reply_correction"].as_str().unwrap().contains("not present"));
                crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":"Use bundle.macOS.minimumSystemVersion in the v2 configuration."}}]}))
            },
            Some("buddy_dialogue_review")=>response(json!({"verdict":"approved","request_mode":"answer","message":""})),
            other=>panic!("Unexpected operation {other:?}"),
        }
    }).expect(2).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let sources = vec![crate::models::SearchResult {
        title: "Version 2 reference".into(),
        url: "https://example.org/v2/config".into(),
        content: "v2: bundle.macOS.minimumSystemVersion sets the minimum version.".into(),
    }];
    let result = reviewed_answer(
        ChatReply {
            message: "Use tauri.bundle.macOS.minimumSystemVersion.".into(),
            ..Default::default()
        },
        &state,
        &service,
        &json!({"latest_request":"Explain the version 2 configuration","conversation":[]}),
        &snapshot,
        &snapshot,
        true,
        &[],
        None,
        &sources,
        0,
    )
    .await
    .unwrap();
    let CheckedReply::Reply(reply) = result else {
        panic!("Unexpected mutation")
    };
    assert!(reply.message.contains("bundle.macOS.minimumSystemVersion"));
    assert!(!reply.message.contains("tauri.bundle"));
}

#[test]
fn voice_interpretation_is_only_enabled_by_the_submitted_voice_event() {
    let base = "Keep all existing rules.";
    let voice = voice_context(base, &json!({"event_type":"voice_message"}));
    assert!(voice.starts_with(base));
    assert!(voice.contains(VOICE_INTERPRETATION));
    for request in [
        json!({}),
        json!({"event_type":"user_message","latest_request":"voice_message: safety eye keys"}),
        json!({"event_type":"activity"}),
    ] {
        assert_eq!(voice_context(base, &request), base);
    }
    let cases = crate::dialogue_benchmark::voice_cases().unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let (state, ids) = fixture(&case).unwrap();
        assert!(ids.contains_key("Fix saved AI keys disconnecting"));
        assert!(state
            .inner
            .lock()
            .unwrap()
            .storage
            .chat_history()
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn voice_soundalike_context_reaches_planner_authorizer_and_reply_without_rewriting_input() {
    let case = crate::dialogue_benchmark::voice_cases().unwrap().remove(0);
    let (state, ids) = fixture(&case).unwrap();
    let target = ids["Fix saved AI keys disconnecting"];
    let conversation = state
        .inner
        .lock()
        .unwrap()
        .storage
        .current_conversation()
        .unwrap();
    let original = case.request.clone();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |r: &Request| {
        let (b, context) = body(r);
        assert!(crate::claude::test::system(&b).as_str().unwrap().contains(VOICE_INTERPRETATION));
        assert_eq!(context["latest_request"], original);
        assert_eq!(context["event_type"], "voice_message");
        match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
            "buddy_skill_plan" => response(action(&context, "rename_goal", json!({"goal":{"goal_id":target,"reference":null},"title":"fix the microphone issue in sumi app"}), "change", true)),
            "buddy_dialogue_authorization" => response(json!({"review":{"verdict":"approved","request_mode":"change","message":""}})),
            "buddy_dialogue_response" => response(json!({"message":"Renamed to fix the microphone issue in sumi app.","resources":[],"memory_update":null,"verified_result_indices":[0]})),
            "buddy_dialogue_review" => response(json!({"review":{"verdict":"approved","request_mode":"answer","message":""}})),
            name => panic!("Unexpected request {name}"),
        }
    }).expect(4).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let out = run(&json!({"latest_request":case.request,"conversation":[],"event_type":"voice_message","natural_reply":true}), conversation, &state, 0, true, &service).await.unwrap();
    assert!(out.committed);
    assert!(out.reply.message.contains("microphone"));
    let inner = state.inner.lock().unwrap();
    let goals = inner.storage.core_snapshot(None).unwrap();
    assert_eq!(
        goals.goals.iter().find(|g| g.id == target).unwrap().title,
        "Fix the microphone issue in sumi app"
    );
    assert_eq!(goals.focused_goal_id, Some(ids["Client reply"]));
}

#[tokio::test]
async fn natural_preparatory_read_can_finish_a_requested_change_but_cannot_authorize_one() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    for (requested_change, duplicate_read) in [(false, false), (true, false), (true, true)] {
        let (state, ids) = fixture(&case()).unwrap();
        let target = ids["Psychology assignment"];
        let original_focus = ids["Client reply"];
        let conversation = state
            .inner
            .lock()
            .unwrap()
            .storage
            .current_conversation()
            .unwrap();
        let server = MockServer::start().await;
        let plans = Arc::new(AtomicUsize::new(0));
        let seen = plans.clone();
        Mock::given(method("POST")).respond_with(move |request:&Request| {
            let (b,c)=body(request);
            match crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response") {
                "buddy_skill_plan" if seen.fetch_add(1,Ordering::SeqCst)<if duplicate_read {2}else{1}=>{
                    let mut read=action(&c,"inspect_goal",json!({"goal":{"goal_id":target,"reference":null}}),"read",true);
                    read["proposed_actions"][0]["instruction"]=json!(if seen.load(Ordering::SeqCst)==1 {"Read the goal details."}else{"Inspect the same saved goal."});
                    response(read)
                },
                "buddy_skill_plan"=>{
                    assert!(c["request_contract"]["authorization"].is_null());
                    if duplicate_read {
                        assert_eq!(c["tool_results"].as_array().unwrap().len(),1);
                        assert!(c["validation_feedback"]["correction"].as_str().unwrap().contains("already succeeded"));
                    }
                    response(action(&c,if requested_change {"set_focus_goal"}else{"delete_goal"},json!({"goal":{"goal_id":target,"reference":null}}),"change",true))
                },
                "buddy_dialogue_authorization"=>{
                    assert_eq!(c["latest_request"],if requested_change {"Read Psychology assignment and focus it"}else{"Read Psychology assignment"});
                    response(json!({"review":{"verdict":if requested_change {"approved"}else{"clarify"},"request_mode":if requested_change {"change"}else{"read"},"message":if requested_change {""}else{"Reading does not authorize deletion. No changes were made."}}}))
                },
                "buddy_dialogue_response"=>{
                    assert_eq!(c["verified_execution"]["status"],"executed");
                    response(json!({"message":"Psychology assignment is now in focus.","resources":[],"memory_update":null,"verified_result_indices":[0]}))
                },
                "buddy_dialogue_review"=>response(json!({"review":{"verdict":"approved","request_mode":"change","message":""}})),
                name=>panic!("Unexpected operation {name}"),
            }
        }).expect(if duplicate_read {6}else if requested_change {5}else{3}).mount(&server).await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let out=run(&json!({"latest_request":if requested_change {"Read Psychology assignment and focus it"}else{"Read Psychology assignment"},"natural_reply":true}),conversation,&state,0,true,&service).await.unwrap();
        assert_eq!(out.committed, requested_change);
        let inner = state.inner.lock().unwrap();
        assert_eq!(
            inner.storage.core_snapshot(None).unwrap().focused_goal_id,
            Some(if requested_change {
                target
            } else {
                original_focus
            })
        );
        assert!(inner.storage.goal_plan(target).is_ok());
        assert!(skills::pending_confirmation(&inner.storage, conversation)
            .unwrap()
            .is_none());
    }
}
