use super::*;

#[tokio::test]
async fn send_route_honors_coaching_goal_even_after_external_blocker() {
    use crate::coaching::{chat, Barrier, Control, Reply, Start};
    let mut storage = store();
    let pinned = goal(&mut storage, "Open Cash ISA", "pinned-coaching");
    let unrelated = goal(&mut storage, "Hackathon", "unrelated-coaching");
    let session = chat::start(
        &storage,
        &Start {
            request_id: "start-pinned".into(),
            goal_id: pinned,
            step_id: None,
            conversation_id: storage.current_conversation().unwrap(),
            expected_target_revision: storage.coaching_target(pinned, None).unwrap().revision,
            strategy_id: "tiny_habits".into(),
            strategy_version: 1,
        },
    )
    .unwrap();
    let session = storage
        .reply_coaching(
            session.id,
            session.conversation_id,
            session.revision,
            "external",
            &Control::Reply {
                reply: Reply::Barrier {
                    barrier: Barrier::ExternalDependency,
                }
                .into(),
            },
        )
        .unwrap();
    let state = AppState::new(storage).unwrap();
    state.inner.lock().unwrap().status.mock_ai = true;
    let reply = send_message(
        "But I need to do this today".into(),
        None,
        Some(unrelated),
        Some(session.conversation_id),
        None,
        &state,
        false,
    )
    .await
    .unwrap();
    assert!(reply.message.contains("external blocker"));
    assert!(!reply.message.contains("Hackathon"));
    let inner = state.inner.lock().unwrap();
    let history = inner
        .storage
        .chat_history_of(session.conversation_id)
        .unwrap();
    assert_eq!(history.len(), 4);
    assert!(history
        .iter()
        .all(|message| message.goal_id == Some(pinned)));
    assert_eq!(
        inner
            .storage
            .coaching_session(session.id, session.conversation_id)
            .unwrap(),
        session
    );
}

#[tokio::test]
async fn audit_regression_repairs_wrong_operation_and_missing_quote_without_model_receipts() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use wiremock::{matchers::method, Mock, MockServer, Request};
    let server = MockServer::start().await;
    let renamed = Arc::new(AtomicUsize::new(0));
    let attempt = renamed.clone();
    Mock::given(method("POST")).respond_with(move |r:&Request| {
        let body:Value=serde_json::from_slice(&r.body).unwrap();
        assert_eq!(crate::claude::test::operation(&body),"buddy_skill_plan");
        let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        let latest=context["latest_request"].as_str().unwrap();
        let goal=json!({"goal_id":context["state"]["goals"][0]["goal_id"],"reference":null});
        let (name,input)=match latest {
            "Create an interview goal" => ("create_goal",json!({"title":"Interview","steps":["Read vacancy","Practice"]})),
            "Первый отметь готовым." => ("complete_step",json!({"goal":goal,"step":{"step_id":null,"ordinal":1,"reference":null}})),
            "Переименуй цель «Interview» в «Ready»." if attempt.fetch_add(1,Ordering::SeqCst)==0 => ("reopen_goal",json!({"goal":goal})),
            "Переименуй цель «Interview» в «Ready»." => ("rename_goal",json!({"goal":goal,"title":"Ready"})),
            _=>panic!("Unexpected synthetic request"),
        };
        let plan=json!({"kind":"actions","actions":[{"name":name,"input":input,"instruction":null}],"state_version":context["state"]["state_version"],"clarification":null});
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(plan).to_string()}}]}))
    }).expect(4).mount(&server).await;
    let s = AppState::new(store()).unwrap();
    let service = Service {
        client: &s.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    for text in [
        "Create an interview goal",
        "Первый отметь готовым.",
        "Переименуй цель «Interview» в «Ready».",
    ] {
        let (c, v) = {
            let i = s.inner.lock().unwrap();
            (
                i.storage.current_conversation().unwrap(),
                i.privacy_revision,
            )
        };
        let result = structured_chat_actions_with_service(
            &json!({"latest_request":text,"conversation":[]}),
            c,
            &s,
            v,
            true,
            &service,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.2);
        if text.starts_with("Переименуй") {
            assert_eq!(result.0, "Renamed the goal to \"Ready\".");
        }
    }
    let i = s.inner.lock().unwrap();
    let g = &i.storage.core_snapshot(None).unwrap().goals[0];
    assert_eq!(g.title, "Ready");
    assert!(g.plan.steps[0].done);
    assert!(!g.plan.steps[1].done);
}

#[tokio::test]
async fn structured_skill_orchestration_uses_real_backend_result_for_reply() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request:&Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        let output=if crate::claude::test::operation(&body)=="buddy_skill_plan" {
            assert_eq!(context["registry"].as_array().unwrap().len(),crate::skills::registry().len());
            json!({"kind":"actions","state_version":context["state"]["state_version"],"actions":[{"name":"create_goal","input":{"title":"Prepare for interview","steps":[]},"instruction":"Create an interview goal"}],"clarification":null})
        } else {
            assert_eq!(context["result"]["status"],"executed");
            assert!(context["result"]["results"][0]["goal_id"].as_i64().unwrap()>0);
            json!({"message":"Created your interview goal."})
        };
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(output).to_string()}}]}))
    }).expect(1).mount(&server).await;
    let storage = store();
    let conversation = storage.current_conversation().unwrap();
    let state = AppState::new(storage).unwrap();
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let reply = structured_chat_actions_with_service(
        &json!({"latest_request":"Create an interview goal","conversation":[]}),
        conversation,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(reply
        .0
        .starts_with("Created \"Prepare for interview\" and added it to Today."));
    assert!(reply.0.contains("- Status: open"));
    assert!(reply.0.contains("- Area: General"));
    assert_eq!(reply.1, 1);
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .goals[0]
            .title,
        "Prepare for interview"
    );
}

#[tokio::test]
async fn structured_skill_orchestration_discards_plan_when_privacy_changes() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    let storage = store();
    let conversation = storage.current_conversation().unwrap();
    let state = std::sync::Arc::new(AppState::new(storage).unwrap());
    let server = MockServer::start().await;
    let captured = state.clone();
    Mock::given(method("POST")).respond_with(move |request:&Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        captured.inner.lock().unwrap().privacy_revision+=1;
        let plan=json!({"kind":"actions","state_version":context["state"]["state_version"],"actions":[{"name":"create_goal","input":{"title":"Must not be created","steps":[]},"instruction":"Create a goal"}],"clarification":null});
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(plan).to_string()}}]}))
    }).expect(1).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    assert!(structured_chat_actions_with_service(
        &json!({"latest_request":"Create a goal","conversation":[]}),
        conversation,
        &state,
        0,
        true,
        &service
    )
    .await
    .is_err());
    assert!(state
        .inner
        .lock()
        .unwrap()
        .storage
        .core_snapshot(None)
        .unwrap()
        .goals
        .is_empty());
}

#[tokio::test]
async fn structured_skill_orchestration_returns_clarification_without_executing() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request:&Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        let plan=json!({"kind":"clarification","state_version":context["state"]["state_version"],"actions":[],"clarification":"Which interview goal do you mean?"});
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(plan).to_string()}}]}))
    }).expect(1).mount(&server).await;
    let mut storage = store();
    goal(&mut storage, "Interview", "one");
    goal(&mut storage, "Interview", "two");
    let conversation = storage.current_conversation().unwrap();
    let state = AppState::new(storage).unwrap();
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let reply = structured_chat_actions_with_service(
        &json!({"latest_request":"Добавь туда шаг","conversation":[]}),
        conversation,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(reply.0, "Which interview goal do you mean?");
    assert!(state
        .inner
        .lock()
        .unwrap()
        .storage
        .core_snapshot(None)
        .unwrap()
        .goals
        .iter()
        .all(|g| g.plan.steps.is_empty()));
}

#[tokio::test]
async fn committed_action_keeps_authoritative_receipt_when_reply_sharing_changes() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    let storage = store();
    let conversation = storage.current_conversation().unwrap();
    let state = std::sync::Arc::new(AppState::new(storage).unwrap());
    let server = MockServer::start().await;
    let captured = state.clone();
    Mock::given(method("POST")).respond_with(move |request:&Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        let response=if crate::claude::test::operation(&body)=="buddy_skill_plan" {
            json!({"kind":"actions","state_version":context["state"]["state_version"],"actions":[{"name":"create_goal","input":{"title":"Interview","steps":[]},"instruction":"Create a goal"}],"clarification":null})
        } else {
            captured.inner.lock().unwrap().privacy_revision+=1;
            json!({"message":"This stale wording must be discarded"})
        };
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(response).to_string()}}]}))
    }).expect(1).mount(&server).await;
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let reply = structured_chat_actions_with_service(
        &json!({"latest_request":"Create a goal","conversation":[]}),
        conversation,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(reply.2);
    assert!(reply.0.contains("Created \"Interview\""));
    assert!(!reply.0.contains("stale wording"));
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
        1
    );
}

fn store() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}
#[test]
fn conversations_isolate_messages_and_delete_only_the_selected_history() {
    let mut storage = store();
    let first = storage.current_conversation().unwrap();
    let reply = ChatReply {
        message: "Answer".into(),
        resources: vec![],
        ..Default::default()
    };
    storage
        .save_chat_turn("First question", &reply, None)
        .unwrap();
    let second = storage.create_conversation().unwrap();
    assert_ne!(first, second);
    assert!(storage.chat_history().unwrap().is_empty());
    for _ in 0..60 {
        storage
            .save_chat_turn("Second question", &reply, None)
            .unwrap();
    }
    assert_eq!(storage.chat_history().unwrap().len(), 100);
    storage.select_conversation(first).unwrap();
    assert_eq!(storage.chat_history().unwrap()[0].text, "First question");
    assert_eq!(storage.chat_history().unwrap().len(), 2);
    storage.delete_conversation(first).unwrap();
    assert_eq!(storage.current_conversation().unwrap(), second);
    assert_eq!(storage.chat_history().unwrap().len(), 100);
    assert!(storage.select_conversation(first).is_err());
    storage.purge_history(None).unwrap();
    assert!(storage.chat_history().unwrap().is_empty());
    assert!(storage
        .conversations()
        .unwrap()
        .iter()
        .all(|item| item.title == "New chat"));
}
#[test]
fn legacy_chat_migration_preserves_turns_and_is_idempotent() {
    let mut storage = store();
    let reply = ChatReply {
        message: "Earlier answer".into(),
        resources: vec![],
        ..Default::default()
    };
    storage
        .save_chat_turn("Earlier question", &reply, None)
        .unwrap();
    storage
        .connection
        .execute("DELETE FROM buddy_conversation_messages", [])
        .unwrap();
    storage
        .connection
        .execute("DELETE FROM buddy_conversations", [])
        .unwrap();
    storage.initialize().unwrap();
    storage.initialize().unwrap();
    assert_eq!(storage.conversations().unwrap().len(), 1);
    assert_eq!(storage.chat_history().unwrap().len(), 2);
    assert_eq!(storage.chat_history().unwrap()[0].text, "Earlier question");
}
#[test]
fn conversation_survives_reopening_the_database_without_an_active_goal() {
    let path = std::env::temp_dir().join(format!(
        "buddy-chat-{}-{}.sqlite",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    {
        let mut storage = Storage::open(&path).unwrap();
        storage
            .save_chat_turn(
                "A question without a goal",
                &ChatReply {
                    message: "A helpful answer".into(),
                    resources: vec![],
                    ..Default::default()
                },
                None,
            )
            .unwrap();
    }
    {
        let storage = Storage::open(&path).unwrap();
        assert!(storage.goal().unwrap().is_none());
        let messages = storage.chat_history().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text, "A question without a goal");
        assert_eq!(messages[1].text, "A helpful answer");
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn older_chat_facts_survive_display_limits_and_database_reopen() {
    let path = std::env::temp_dir().join(format!(
        "buddy-retained-{}.sqlite",
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let first;
    {
        let mut storage = Storage::open(&path).unwrap();
        first = storage.current_conversation().unwrap();
        let reply = ChatReply {
            message: "Understood".into(),
            ..Default::default()
        };
        storage
            .save_chat_turn(
                "Маяк: бюджет 18000, аудитория — владельцы кафе",
                &reply,
                None,
            )
            .unwrap();
        for index in 0..65 {
            storage
                .save_chat_turn(&format!("Synthetic turn {index}"), &reply, None)
                .unwrap();
        }
        let second = storage.create_conversation().unwrap();
        storage
            .save_chat_turn("A different chat has a different budget", &reply, None)
            .unwrap();
        assert_ne!(first, second);
    }
    {
        let storage = Storage::open(&path).unwrap();
        assert_eq!(storage.chat_history_of(first).unwrap().len(), 100);
        let count: i64 = storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM buddy_conversation_messages WHERE conversation_id=?1",
                [first],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 132,
            "The UI budget must never purge retained messages"
        );
        let page = storage.inspect_conversation(first, "МАЯК", None).unwrap();
        assert_eq!(page["messages"].as_array().unwrap().len(), 1);
        assert!(page["messages"][0]["text"]
            .as_str()
            .unwrap()
            .contains("18000"));
        assert!(storage
            .inspect_conversation(first, "different budget", None)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn retained_chat_pages_are_bounded_ordered_and_do_not_skip_messages() {
    let mut storage = store();
    let conversation = storage.current_conversation().unwrap();
    let reply = ChatReply {
        message: "🙂".repeat(6000),
        ..Default::default()
    };
    for index in 0..12 {
        storage
            .save_chat_turn(&format!("Turn {index}"), &reply, None)
            .unwrap();
    }
    let mut before = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = storage
            .inspect_conversation(conversation, "", before)
            .unwrap();
        assert!(page["messages"].to_string().len() < 25 * 1024);
        let mut previous = 0;
        for message in page["messages"].as_array().unwrap() {
            let id = message["id"].as_i64().unwrap();
            assert!(id > previous);
            assert!(seen.insert(id));
            previous = id;
        }
        if page["has_more"] == false {
            break;
        }
        let next = page["next_before_message_id"].as_i64().unwrap();
        assert!(before.is_none_or(|old| next < old));
        before = Some(next);
    }
    assert_eq!(seen.len(), 24);
    assert!(storage
        .inspect_conversation(conversation, "", Some(-1))
        .is_err());
    assert!(storage
        .inspect_conversation(conversation, &"x".repeat(201), None)
        .is_err());
    storage.purge_history(None).unwrap();
    assert!(storage
        .inspect_conversation(conversation, "", None)
        .is_err());
    assert!(storage.chat_history().unwrap().is_empty());
}
#[test]
fn unknown_duration_does_not_trigger_coaching_and_completed_goals_are_suppressed() {
    let mut storage = store();
    let id = goal(&mut storage, "No estimate", "unknown");
    let now = chrono::Utc::now().timestamp();
    assert!(!crate::companion::stuck_due(&storage, id, now).unwrap());
    storage.connection.execute("INSERT INTO core_goal_details(goal_id,due_at) VALUES(?1,?2) ON CONFLICT(goal_id) DO UPDATE SET due_at=excluded.due_at",params![id,chrono::Utc::now().to_rfc3339()]).unwrap();
    assert!(crate::companion::stuck_due(&storage, id, now + 1).unwrap());
    storage.transition_goal(id, "resume").unwrap();
    storage.transition_goal(id, "complete").unwrap();
    assert!(!crate::companion::stuck_due(&storage, id, now + 1).unwrap());
}
fn goal(store: &mut Storage, title: &str, batch: &str) -> i64 {
    store
        .core_add_goals(
            vec![crate::core::GoalDraft {
                title: title.into(),
                area: "Work".into(),
                steps: vec![],
            }],
            batch,
        )
        .unwrap();
    store
        .core_snapshot(None)
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.title == title)
        .unwrap()
        .id
}
#[test]
fn conversation_is_shared_across_goals_bounded_and_cleared_by_retention() {
    let mut storage = store();
    let id = goal(&mut storage, "First", "first");
    let reply = ChatReply {
        message: "Choose one next step.".into(),
        resources: vec![],
        ..Default::default()
    };
    storage
        .save_chat_turn("What is blocking me?", &reply, Some(id))
        .unwrap();
    storage
        .save_chat_turn("A separate question", &reply, None)
        .unwrap();
    let history = storage.chat_history().unwrap();
    assert_eq!(history.len(), 4);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[1].role, "assistant");
    assert!(history[2].goal_id.is_none());
    for _ in 0..55 {
        storage
            .save_chat_turn("Next request", &reply, Some(id))
            .unwrap();
    }
    let history = storage.chat_history().unwrap();
    assert_eq!(history.len(), 100);
    assert_eq!(history[0].role, "user");
    storage.purge_history(None).unwrap();
    assert!(storage.chat_history().unwrap().is_empty());
}
#[test]
fn chat_sources_cannot_be_invented_and_no_goal_is_changed_by_a_reply() {
    let mut storage = store();
    let id = goal(&mut storage, "Original goal", "original");
    let original = storage.goal_plan(id).unwrap();
    let response = json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"message":"Try a next step.","resources":[{"title":"Invented","url":"https://invented.example/","whyRelevant":"Claims to help"}]}).to_string()}}]});
    let reply = parse_chat(&response, &[]).unwrap();
    assert!(reply.resources.is_empty());
    let invented = json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"message":"Read https://invented.example/","resources":[]}).to_string()}}]});
    assert!(parse_chat(&invented, &[]).is_err());
    storage.save_chat_turn("Help", &reply, Some(id)).unwrap();
    assert_eq!(storage.goal_plan(id).unwrap().revision, original.revision);
    assert_eq!(
        storage.core_snapshot(None).unwrap().goals[0].title,
        "Original goal"
    );
}
#[test]
fn observed_time_reaches_progress_without_double_counting_timer_and_drives_stuck_trigger() {
    let mut storage = store();
    let id = goal(&mut storage, "Work", "work");
    storage.transition_goal(id, "resume").unwrap();
    let day = chrono::Local::now().date_naive().to_string();
    storage
        .connection
        .execute(
            "INSERT INTO core_time(day,goal_id,seconds) VALUES(?1,?2,60)",
            params![day, id],
        )
        .unwrap();
    storage.connection.execute("INSERT INTO usage_daily(day,goal_id,process_name,milliseconds) VALUES(?1,?2,'editor.exe',180000)",params![day,id]).unwrap();
    storage
        .connection
        .execute(
            "INSERT INTO ai_goal_estimates(goal_id,minutes) VALUES(?1,2)",
            [id],
        )
        .unwrap();
    let snapshot = storage.core_snapshot(None).unwrap();
    let progress = &snapshot.summary.goals[0];
    assert_eq!(progress.seconds, 60);
    assert_eq!(progress.tracked_seconds, 180);
    assert_eq!(snapshot.goals[0].tracked_seconds, 180);
    assert!(!storage.coaching_context(id).unwrap()["over_expected"]
        .as_bool()
        .unwrap());
    storage.connection.execute("INSERT INTO goal_relevant_daily(day,goal_id,process_name,milliseconds) VALUES(?1,?2,'editor.exe',180000)",params![day,id]).unwrap();
    assert!(storage.coaching_context(id).unwrap()["over_expected"]
        .as_bool()
        .unwrap());
    assert!(crate::companion::stuck_due(&storage, id, chrono::Utc::now().timestamp()).unwrap());
}
#[test]
fn tracking_requires_both_persistent_request_and_consent_and_keeps_pause_after_goal_switch() {
    let mut storage = store();
    let first = goal(&mut storage, "First", "first");
    let second = goal(&mut storage, "Second", "second");
    storage.transition_goal(first, "resume").unwrap();
    let mut settings = storage.user_settings().unwrap();
    settings.onboarding.completed = true;
    settings.onboarding.tracking_consent = true;
    storage.write_setting("user_settings", &settings).unwrap();
    assert!(crate::tracking::requested(&storage).unwrap());
    storage.write_setting("tracking_requested", &true).unwrap();
    assert!(crate::tracking::requested(&storage).unwrap());
    storage.transition_goal(second, "resume").unwrap();
    assert!(crate::tracking::requested(&storage).unwrap());
    storage.write_setting("tracking_requested", &false).unwrap();
    storage.transition_goal(first, "resume").unwrap();
    assert!(!crate::tracking::requested(&storage).unwrap());
    storage.write_setting("tracking_requested", &true).unwrap();
    settings.onboarding.tracking_consent = false;
    storage.write_setting("user_settings", &settings).unwrap();
    assert!(!crate::tracking::requested(&storage).unwrap());
}

#[tokio::test]
#[ignore = "Uses configured Claude/Tavily accounts with synthetic data; provider charges may apply"]
async fn live_ai_goal_and_chat_smoke() {
    std::env::set_var("BUDDY_LIVE_DIAGNOSTICS", "true");
    let _ = dotenvy::from_path(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env"));
    let mut storage = store();
    let id = goal(
        &mut storage,
        "Create a Tauri desktop onboarding flow",
        "live-smoke",
    );
    storage.ensure_today_focus().unwrap();
    let state = AppState::new(storage).unwrap();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.status.mock_ai = false;
    }
    let input = {
        let inner = state.inner.lock().unwrap();
        crate::automatic_goals::input(
            inner
                .storage
                .core_snapshot(None)
                .unwrap()
                .goals
                .iter()
                .find(|g| g.id == id)
                .unwrap(),
        )
    };
    let result = crate::goal_analysis::analyze_goal(input, true, &state)
        .await
        .map_err(|e| {
            eprintln!(
                "Live goal failure category: {}",
                if e.contains("incomplete") {
                    "incomplete"
                } else if e.contains("goal suggestions") {
                    "suggestion_schema"
                } else if e.contains("invalid goal suggestion fields") {
                    "suggestion_bounds"
                } else if e.contains("classification") {
                    "classifier"
                } else {
                    "other"
                }
            );
            crate::goal_analysis::user_error("live_analysis", &e)
        })
        .unwrap();
    assert!(
        (3..=5).contains(&result.suggested_steps.len()),
        "Live goal suggestions did not contain 3–5 steps"
    );
    assert!(
        result.improved_title.is_some(),
        "Live title suggestion missing"
    );
    assert!(
        !result.resources.is_empty(),
        "No grounded web sources returned"
    );
    let reply = send_message(
        "Help me start the first onboarding step".into(),
        None,
        None,
        None,
        None,
        &state,
        false,
    )
    .await
    .map_err(|e| crate::goal_analysis::user_error("live_chat", &e))
    .unwrap();
    assert!(!reply.message.trim().is_empty());
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .chat_history()
            .unwrap()
            .len(),
        2
    );
    println!(
        "Live synthetic smoke passed: title, {} steps, {} grounded sources, persistent chat",
        result.suggested_steps.len(),
        result.resources.len()
    );
}
#[test]
fn chat_actions_add_goals_and_steps_and_report_exact_results() {
    let mut storage = store();
    let id = goal(&mut storage, "Update the app", "chat-actions");
    let action =
        |kind: &str, goal_id: Option<i64>, title: Option<&str>, steps: &[&str]| ChatAction {
            kind: kind.into(),
            goal_id,
            title: title.map(str::to_string),
            steps: steps.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
    let lines = apply_chat_actions(
        &mut storage,
        Some(id),
        &[
            action(
                "add_steps",
                None,
                None,
                &["fix the toggles", "Fix the toggles", "ship it"],
            ),
            action(
                "add_goal",
                None,
                Some("publish a website"),
                &["pick a domain", "write the pages"],
            ),
            action("add_steps", Some(9999), None, &["x"]),
            action("add_steps", Some(id), None, &["ship it"]),
        ],
    );
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("Added 2 steps to"));
    assert!(lines[1].contains("Publish a website") && lines[1].contains("2 steps"));
    assert!(lines[2].starts_with("Couldn't"));
    assert!(lines[3].starts_with("Nothing new"));
    let snapshot = storage.core_snapshot(None).unwrap();
    let existing = snapshot.goals.iter().find(|g| g.id == id).unwrap();
    let texts: Vec<_> = existing
        .plan
        .steps
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(texts.contains(&"Fix the toggles") && texts.contains(&"Ship it"));
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.to_lowercase() == "fix the toggles")
            .count(),
        1
    );
    let created = snapshot
        .goals
        .iter()
        .find(|g| g.title == "Publish a website")
        .unwrap();
    assert_eq!(created.plan.steps.len(), 2);
    assert_eq!(created.plan.steps[0].text, "Pick a domain");
}
#[test]
fn add_requests_are_classified_into_strict_intents() {
    let ok = |body: Value| json!({"choices":[{"finish_reason":"stop","message":{"content":body.to_string()}}]});
    let intent = parse_intent(&ok(
        json!({"intent":"add_steps","goal_id":3,"title":null,"steps":["One","Two"]}),
    ))
    .unwrap();
    assert_eq!(intent.intent, "add_steps");
    assert_eq!(intent.goal_id, Some(3));
    assert_eq!(intent.steps.len(), 2);
    assert!(parse_intent(&ok(
        json!({"intent":"format_disk","goal_id":3,"title":null,"steps":[]})
    ))
    .is_err());
    assert!(parse_intent(&ok(
        json!({"intent":"none","goal_id":null,"title":null,"steps":[],"extra":1})
    ))
    .is_err());
}
#[test]
fn chat_goals_go_to_today_and_are_never_duplicated() {
    let mut storage = store();
    let existing = goal(&mut storage, "Existing goal", "chat-today");
    let action =
        |kind: &str, goal_id: Option<i64>, title: Option<&str>, steps: &[&str]| ChatAction {
            kind: kind.into(),
            goal_id,
            title: title.map(str::to_string),
            steps: steps.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
    let lines = apply_chat_actions(
        &mut storage,
        Some(existing),
        &[action(
            "add_goal",
            None,
            Some("fix the chats"),
            &["send with enter", "clear the input"],
        )],
    );
    assert!(lines[0].contains("Added the goal \"Fix the chats\" with 2 steps"));
    assert!(lines[0].contains("Also added it to Today."));
    let snapshot = storage.core_snapshot(None).unwrap();
    let created = snapshot
        .goals
        .iter()
        .find(|g| g.title == "Fix the chats")
        .unwrap();
    assert!(snapshot.today.contains(&created.id));
    // Asking again for the same goal does not create a copy; it only fills in what is missing.
    let again = apply_chat_actions(
        &mut storage,
        Some(existing),
        &[action(
            "add_goal",
            None,
            Some("Fix the chats"),
            &["send with enter", "keep focus after sending"],
        )],
    );
    assert!(again[0].contains("already in your goals"));
    assert!(again[0].contains("Added 1 new step"));
    let snapshot = storage.core_snapshot(None).unwrap();
    assert_eq!(
        snapshot
            .goals
            .iter()
            .filter(|g| g.title == "Fix the chats")
            .count(),
        1
    );
    // An existing goal can be put on Today by name, and Today has no size limit.
    let one = apply_chat_actions(
        &mut storage,
        None,
        &[action("add_to_today", None, Some("existing goal"), &[])],
    );
    assert!(one[0].contains("Also added it to Today"));
    for title in ["Third goal", "Fourth goal"] {
        apply_chat_actions(
            &mut storage,
            None,
            &[action("add_goal", None, Some(title), &["x"])],
        );
    }
    let snapshot = storage.core_snapshot(None).unwrap();
    assert_eq!(snapshot.today.len(), 4);
    let fourth = snapshot
        .goals
        .iter()
        .find(|g| g.title == "Fourth goal")
        .unwrap();
    assert!(snapshot.today.contains(&fourth.id));
}
#[test]
fn chat_can_rename_edit_complete_and_remove_but_reports_what_it_did() {
    let mut storage = store();
    let id = goal(&mut storage, "Old name", "chat-edit");
    apply_chat_actions(
        &mut storage,
        Some(id),
        &[ChatAction {
            kind: "add_steps".into(),
            steps: vec!["write the intro".into(), "write the outro".into()],
            ..Default::default()
        }],
    );
    let run = |storage: &mut Storage,
               kind: &str,
               title: Option<&str>,
               target: Option<&str>,
               new_text: Option<&str>| {
        apply_chat_actions(
            storage,
            Some(id),
            &[ChatAction {
                kind: kind.into(),
                goal_id: Some(id),
                title: title.map(str::to_string),
                target: target.map(str::to_string),
                new_text: new_text.map(str::to_string),
                ..Default::default()
            }],
        )
        .remove(0)
    };
    let steps = |storage: &Storage| -> Vec<(String, bool)> {
        storage.core_snapshot(None).unwrap().goals[0]
            .plan
            .steps
            .iter()
            .map(|s| (s.text.clone(), s.done))
            .collect()
    };
    assert!(
        run(&mut storage, "rename_goal", Some("new name"), None, None)
            .contains("Renamed \"Old name\" to \"New name\"")
    );
    assert_eq!(
        storage.core_snapshot(None).unwrap().goals[0].title,
        "New name"
    );
    assert!(run(
        &mut storage,
        "edit_step",
        None,
        Some("intro"),
        Some("write a better intro")
    )
    .contains("Changed the step"));
    assert!(run(&mut storage, "complete_step", None, Some("outro"), None).contains("Marked"));
    assert_eq!(
        steps(&storage),
        vec![
            ("Write a better intro".to_string(), false),
            ("Write the outro".to_string(), true)
        ]
    );
    // Ambiguous or unknown steps are reported, never guessed.
    assert!(
        run(&mut storage, "delete_step", None, Some("write"), None).starts_with("More than one")
    );
    assert!(
        run(&mut storage, "delete_step", None, Some("conclusion"), None).starts_with("I couldn't")
    );
    assert!(run(
        &mut storage,
        "delete_step",
        None,
        Some("write the outro"),
        None
    )
    .contains("Removed the step"));
    assert_eq!(steps(&storage).len(), 1);
    // Completing and reopening a goal.
    assert!(run(&mut storage, "complete_goal", None, None, None).contains("Completed"));
    assert_eq!(
        storage.core_snapshot(None).unwrap().goals[0].status,
        "completed"
    );
    assert!(run(&mut storage, "reopen_goal", None, None, None).contains("Reopened"));
    assert_eq!(storage.core_snapshot(None).unwrap().goals[0].status, "open");
}
#[test]
fn chat_deletes_at_once_and_can_restore_the_goal_with_its_steps() {
    let mut storage = store();
    let id = goal(&mut storage, "Delete me", "chat-delete");
    apply_chat_actions(
        &mut storage,
        Some(id),
        &[ChatAction {
            kind: "add_steps".into(),
            steps: vec!["first step".into(), "second step".into()],
            ..Default::default()
        }],
    );
    let mut plan = storage.core_snapshot(None).unwrap().goals[0].plan.clone();
    plan.steps[0].done = true;
    storage
        .core_save_goal_details("Delete me", "General", plan, None)
        .unwrap();
    let first_step = storage.core_snapshot(None).unwrap().goals[0].plan.steps[0]
        .id
        .clone();
    storage.connection.execute("INSERT INTO goal_relevant_daily(day,goal_id,process_name,milliseconds) VALUES('2026-10-12',?1,'editor',120000)", [id]).unwrap();
    storage.connection.execute("INSERT INTO goal_relevant_step_daily(day,goal_id,step_id,milliseconds) VALUES('2026-10-12',?1,?2,120000)", params![id, first_step]).unwrap();
    storage
        .connection
        .execute(
            "INSERT INTO core_time(day,goal_id,seconds) VALUES('2026-10-12',?1,300)",
            [id],
        )
        .unwrap();
    let line = apply_chat_actions(
        &mut storage,
        None,
        &[ChatAction {
            kind: "delete_goal".into(),
            goal_id: Some(id),
            ..Default::default()
        }],
    )
    .remove(0);
    assert!(line.starts_with("Deleted \"Delete me\""));
    assert!(storage.core_snapshot(None).unwrap().goals.is_empty());
    assert_eq!(deleted_goals(&storage).len(), 1);
    let line = apply_chat_actions(
        &mut storage,
        None,
        &[ChatAction {
            kind: "restore_goal".into(),
            ..Default::default()
        }],
    )
    .remove(0);
    assert!(line.starts_with("Restored \"Delete me\" with 2 steps"));
    let snapshot = storage.core_snapshot(None).unwrap();
    assert_eq!(snapshot.goals.len(), 1);
    let steps: Vec<_> = snapshot.goals[0]
        .plan
        .steps
        .iter()
        .map(|s| (s.text.as_str(), s.done))
        .collect();
    assert_eq!(steps, vec![("First step", true), ("Second step", false)]);
    // The recorded time came back with the goal, tied to the same step ids.
    let new_id = snapshot.goals[0].id;
    assert_eq!(storage.relevant_seconds(new_id, None).unwrap(), 120);
    assert_eq!(snapshot.goals[0].focused_seconds, 300);
    let times = storage.step_times(new_id, "2026-10-12").unwrap();
    assert_eq!(times.len(), 1);
    assert_eq!(
        (times[0].step_id.as_str(), times[0].seconds),
        (first_step.as_str(), 120)
    );
    assert!(deleted_goals(&storage).is_empty());
    let again = apply_chat_actions(
        &mut storage,
        None,
        &[ChatAction {
            kind: "restore_goal".into(),
            ..Default::default()
        }],
    )
    .remove(0);
    assert_eq!(again, "There is no deleted goal to restore.");
}
#[test]
fn deleted_goal_backups_follow_history_retention() {
    let mut storage = store();
    let id = goal(&mut storage, "Temporary", "chat-retention");
    apply_chat_actions(
        &mut storage,
        None,
        &[ChatAction {
            kind: "delete_goal".into(),
            goal_id: Some(id),
            ..Default::default()
        }],
    );
    assert_eq!(deleted_goals(&storage), vec!["Temporary".to_string()]);
    // Backups older than 30 days disappear on their own.
    storage
        .connection
        .execute(
            "UPDATE deleted_goal_backups SET deleted_at=deleted_at-?1",
            [KEEP_DELETED_SECONDS + 10],
        )
        .unwrap();
    assert!(deleted_goals(&storage).is_empty());
    // Deleting all history also removes them.
    let again = goal(&mut storage, "Another", "chat-retention-2");
    apply_chat_actions(
        &mut storage,
        None,
        &[ChatAction {
            kind: "delete_goal".into(),
            goal_id: Some(again),
            ..Default::default()
        }],
    );
    assert_eq!(deleted_goals(&storage).len(), 1);
    storage.purge_history(None).unwrap();
    assert!(deleted_goals(&storage).is_empty());
}
#[test]
fn chat_titles_name_the_topic_and_are_cleaned_up() {
    let ok = |body: Value| json!({"choices":[{"finish_reason":"stop","message":{"content":body.to_string()}}]});
    assert_eq!(
        parse_title(&ok(json!({"title":"\"apply for 10 jobs on Jumpstart.\""}))).unwrap(),
        "Apply for 10 jobs on Jumpstart"
    );
    assert!(parse_title(&ok(json!({"title":"   "}))).is_err());
    assert!(parse_title(&ok(json!({"title":"x".repeat(61)}))).is_err());
    assert!(parse_title(&ok(json!({"title":"ok","extra":1}))).is_err());
    let mut storage = store();
    let conversation = storage.current_conversation().unwrap();
    storage
        .save_chat_turn(
            "add a step to fix phrasing in the chat",
            &ChatReply {
                message: "Added.".into(),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    storage
        .set_conversation_title(conversation, "Fixing chat phrasing")
        .unwrap();
    let titles: Vec<_> = storage
        .conversations()
        .unwrap()
        .into_iter()
        .map(|c| c.title)
        .collect();
    assert_eq!(titles, vec!["Fixing chat phrasing".to_string()]);
}
#[test]
fn desktop_chat_resumes_before_ten_minutes_and_starts_empty_at_the_boundary() {
    let mut storage = store();
    let first = storage.current_conversation().unwrap();
    storage
        .save_chat_turn("Keep this topic", &ChatReply::default(), None)
        .unwrap();
    let at = chrono::Utc::now();
    storage
        .connection
        .execute(
            "UPDATE buddy_conversations SET updated_at=?1 WHERE id=?2",
            params![at.to_rfc3339(), first],
        )
        .unwrap();
    assert!(!storage
        .resume_companion_conversation(at + chrono::Duration::seconds(599))
        .unwrap());
    assert_eq!(storage.current_conversation().unwrap(), first);
    assert!(storage
        .resume_companion_conversation(at + chrono::Duration::minutes(10))
        .unwrap());
    let second = storage.current_conversation().unwrap();
    assert_ne!(first, second);
    assert!(storage.chat_history().unwrap().is_empty());
    assert_eq!(storage.chat_history_of(first).unwrap().len(), 2);
    // Reopening an empty chat must not fill the list with empty threads.
    assert!(!storage
        .resume_companion_conversation(at + chrono::Duration::days(2))
        .unwrap());
    assert_eq!(storage.current_conversation().unwrap(), second);
    // Explicitly opening an old chat in the workspace still shows its history.
    storage.select_conversation(first).unwrap();
    assert_eq!(storage.chat_history().unwrap()[0].text, "Keep this topic");
}

#[test]
fn desktop_chat_inactivity_survives_restart_and_new_messages_restart_the_clock() {
    let path = std::env::temp_dir().join(format!(
        "buddy-desktop-chat-{}.sqlite",
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let first;
    {
        let mut storage = Storage::open(&path).unwrap();
        first = storage.current_conversation().unwrap();
        storage
            .save_chat_turn("Yesterday's topic", &ChatReply::default(), None)
            .unwrap();
        let yesterday = chrono::Utc::now() - chrono::Duration::days(2);
        storage
            .connection
            .execute(
                "UPDATE buddy_conversations SET updated_at=?1 WHERE id=?2",
                params![yesterday.to_rfc3339(), first],
            )
            .unwrap();
    }
    let mut storage = Storage::open(&path).unwrap();
    assert!(storage
        .resume_companion_conversation(chrono::Utc::now())
        .unwrap());
    let current = storage.current_conversation().unwrap();
    assert_ne!(first, current);
    storage
        .save_question_to(current, "Another goal", None)
        .unwrap();
    assert!(!storage
        .resume_companion_conversation(chrono::Utc::now() + chrono::Duration::minutes(9))
        .unwrap());
    storage
        .save_answer_to(current, &ChatReply::default(), None)
        .unwrap();
    assert!(!storage
        .resume_companion_conversation(chrono::Utc::now())
        .unwrap());
    drop(storage);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn explicit_new_chat_starts_a_conversation_only_when_the_current_one_has_messages() {
    let mut storage = store();
    let first = storage.current_conversation().unwrap();
    assert!(!storage.start_fresh_conversation().unwrap());
    assert_eq!(storage.current_conversation().unwrap(), first);
    storage
        .save_chat_turn(
            "Hello",
            &ChatReply {
                message: "Hi".into(),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    assert!(storage.start_fresh_conversation().unwrap());
    let second = storage.current_conversation().unwrap();
    assert_ne!(first, second);
    assert!(storage.chat_history().unwrap().is_empty());
    // The earlier chat is kept and still listed, so it stays available in the app.
    assert_eq!(storage.conversations().unwrap().len(), 2);
    assert!(!storage.start_fresh_conversation().unwrap());
}
#[test]
fn chat_changes_need_a_quoted_instruction_and_questions_are_not_commands() {
    let intent = |kind: &str, quote: Option<&str>| ChatIntent {
        intent: kind.into(),
        goal_id: Some(1),
        title: None,
        steps: vec![],
        target: Some("x".into()),
        new_text: None,
        quote: quote.map(str::to_string),
    };
    let question = "i want to finish the japanese mascots goal, what do i need for it";
    // The model must quote the instruction; a made-up or missing quote is ignored.
    assert!(!grounded(&intent("complete_step", None), question));
    assert!(!grounded(
        &intent("complete_step", Some("mark it as done")),
        question
    ));
    assert!(!grounded(
        &intent("complete_step", Some("what do i need for it")),
        question
    ));
    assert!(grounded(&intent("none", None), question));
    // A real instruction quoted from the message passes, also as a polite question.
    assert!(grounded(
        &intent("complete_step", Some("mark the mascots step as done")),
        "Please mark the mascots step as done"
    ));
    assert!(grounded(
        &intent("add_steps", Some("add a step")),
        "can you add a step to fix the chat?"
    ));
    assert!(!grounded(
        &intent("delete_goal", Some("delete")),
        "should i delete?"
    ));
}
#[test]
fn an_answer_is_saved_in_the_chat_it_was_asked_in_even_after_a_new_chat_is_started() {
    let mut storage = store();
    let first = storage.current_conversation().unwrap();
    // The user asks in the first chat and, while sumi is still answering, starts another chat.
    let second = storage.create_conversation().unwrap();
    assert_eq!(storage.current_conversation().unwrap(), second);
    let reply = ChatReply {
        message: "Here is the answer.".into(),
        ..Default::default()
    };
    storage
        .save_chat_turn_to(first, "My question", &reply, None)
        .unwrap();
    let texts = |storage: &Storage, id: i64| -> Vec<String> {
        storage
            .chat_history_of(id)
            .unwrap()
            .into_iter()
            .map(|m| m.text)
            .collect()
    };
    assert_eq!(
        texts(&storage, first),
        vec!["My question", "Here is the answer."]
    );
    assert!(texts(&storage, second).is_empty());
    // A chat that was deleted meanwhile does not receive the answer.
    storage.delete_conversation(first).unwrap();
    assert!(storage
        .save_chat_turn_to(first, "Late", &reply, None)
        .is_err());
    assert!(storage.conversation_exists(second));
}

#[test]
fn searching_chats_matches_titles_and_message_text() {
    let mut storage = store();
    let first = storage.current_conversation().unwrap();
    let reply = ChatReply {
        message: "Try the green mascot".into(),
        resources: vec![],
        ..Default::default()
    };
    storage
        .save_chat_turn("How do I plan my 100% week?", &reply, None)
        .unwrap();
    storage
        .set_conversation_title(first, "Weekly plan")
        .unwrap();
    let second = storage.create_conversation().unwrap();
    assert_eq!(storage.search_conversations("weekly").unwrap(), vec![first]);
    assert_eq!(storage.search_conversations("MASCOT").unwrap(), vec![first]);
    assert_eq!(storage.search_conversations("100%").unwrap(), vec![first]);
    assert!(storage.search_conversations("zzz").unwrap().is_empty());
    assert!(!storage
        .search_conversations("chat")
        .unwrap()
        .contains(&first));
    assert!(storage
        .search_conversations("chat")
        .unwrap()
        .contains(&second));
}

#[test]
fn a_question_is_kept_even_when_the_answer_never_arrives() {
    let mut storage = store();
    let chat = storage.current_conversation().unwrap();
    storage
        .save_question_to(chat, "Write me a prompt", None)
        .unwrap();
    let history = storage.chat_history_of(chat).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].text, "Write me a prompt");
    assert_eq!(
        storage
            .conversations()
            .unwrap()
            .iter()
            .find(|c| c.id == chat)
            .unwrap()
            .title,
        "Write me a prompt"
    );
}

#[test]
fn empty_chats_disappear_when_the_user_moves_on() {
    let mut storage = store();
    let first = storage.current_conversation().unwrap();
    storage.save_question_to(first, "Hello", None).unwrap();
    assert!(storage.start_fresh_conversation().unwrap());
    let empty = storage.current_conversation().unwrap();
    assert_ne!(empty, first);
    // Asking for another new chat while this one is empty reuses it.
    assert!(!storage.start_fresh_conversation().unwrap());
    assert_eq!(storage.conversations().unwrap().len(), 2);
    storage.select_conversation(first).unwrap();
    let ids: Vec<_> = storage
        .conversations()
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, vec![first]);
    // Deleting the last chat leaves one fresh empty chat to type in.
    storage.delete_conversation(first).unwrap();
    assert_eq!(storage.conversations().unwrap().len(), 1);
}
#[tokio::test]
async fn chat_uses_contextual_memory_and_memory_off_disables_recall_without_deleting_history() {
    let mut storage = store();
    let id = goal(&mut storage, "Hackathon", "memory-chat");
    let memory = storage
        .create_memory(crate::memory::MemoryInput {
            memory_type: crate::memory::MemoryType::ExplicitFact,
            scope: crate::memory::Scope::goal(id),
            content: "Draft reviewed yesterday".into(),
        })
        .unwrap();
    let state = AppState::new(storage).unwrap();
    state.inner.lock().unwrap().status.mock_ai = true;
    let reply = send_message(
        "Where did I stop with Hackathon?".into(),
        None,
        Some(id),
        None,
        None,
        &state,
        false,
    )
    .await
    .unwrap();
    assert!(reply.message.contains("AI_MOCK"));
    {
        let inner = state.inner.lock().unwrap();
        let metadata: String = inner
            .storage
            .connection
            .query_row(
                "SELECT metadata FROM memory_retrieval_debug WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let metadata: Value = serde_json::from_str(&metadata).unwrap();
        assert!(metadata["selected"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == memory.id));
        let mut controls = inner.storage.memory_controls().unwrap();
        controls.enabled = false;
        inner.storage.set_memory_controls(&controls).unwrap();
    }
    send_message(
        "Where did I stop?".into(),
        None,
        Some(id),
        None,
        None,
        &state,
        false,
    )
    .await
    .unwrap();
    let inner = state.inner.lock().unwrap();
    let metadata: String = inner
        .storage
        .connection
        .query_row(
            "SELECT metadata FROM memory_retrieval_debug WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let metadata: Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["disabled"], true);
    assert_eq!(metadata["passed"], 0);
    assert!(inner.storage.chat_history().unwrap().len() >= 4);
    assert!(inner.storage.memory(memory.id, false).is_ok());
}
#[test]
fn chat_parser_accepts_a_structured_memory_proposal_without_storing_it() {
    let response = json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"message":"Review the suggestion in Settings.","resources":[],"memory_update":{"operation":"store_memory","type":"preference","content":"I prefer concise plans","confidence":0.85,"source":"explicit_user_statement"}}).to_string()}}]});
    let reply = parse_chat(&response, &[]).unwrap();
    assert!(reply.memory_update.is_some());
    assert!(serde_json::to_value(&reply)
        .unwrap()
        .get("memory_update")
        .is_none());
    assert!(store()
        .list_memories(Default::default())
        .unwrap()
        .is_empty());
}
#[tokio::test]
async fn inspect_skill_uses_validated_goal_id_to_retrieve_checkpoint_for_memory_question() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    let mut storage = store();
    let goal_id = goal(&mut storage, "Хакатон", "memory-inspect");
    let other = goal(&mut storage, "Other project", "memory-other");
    storage
        .save_work_context(
            crate::memory::WorkContextInput {
                goal_id,
                step_id: None,
                summary: "Outline finished".into(),
                next_action: Some("Write the pitch".into()),
                status: crate::memory::ContextStatus::Active,
                last_app: None,
                last_window: None,
                waiting_for: None,
            },
            None,
        )
        .unwrap();
    storage
        .create_memory(crate::memory::MemoryInput {
            scope: crate::memory::Scope::goal(other),
            memory_type: crate::memory::MemoryType::ExplicitFact,
            content: "Other confidential project".into(),
        })
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |request:&Request|{
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        let output=if crate::claude::test::operation(&body)=="buddy_skill_plan"{
            json!({"kind":"actions","state_version":context["state"]["state_version"],"actions":[{"name":"inspect_goal","input":{"goal":{"goal_id":goal_id,"reference":null}},"instruction":"Где я остановился с хакатоном?"}],"clarification":null})
        } else {
            assert_eq!(context["personal_memory"]["work_context"]["summary"],"Outline finished");
            assert_eq!(context["personal_memory"]["work_context"]["next_action"],"Write the pitch");
            assert!(!context["personal_memory"].to_string().contains("Other confidential"));
            json!({"message":"The outline is finished. Next: write the pitch."})
        };
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(output).to_string()}}]}))
    }).expect(1).mount(&server).await;
    let conversation = storage.current_conversation().unwrap();
    let state = AppState::new(storage).unwrap();
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let reply = structured_chat_actions_with_service(
        &json!({"latest_request":"Где я остановился с хакатоном?","conversation":[]}),
        conversation,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(reply.0.contains("Write the pitch"));
    assert!(!reply.2);
    assert!(!reply.0.contains("Other confidential"));
}

#[tokio::test]
async fn park_chat_commands_generate_then_validate_then_save_checkpoint() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    for phrase in [
        "Park this",
        "Сохрани, где я остановился",
        "Запомни это, мне надо переключиться",
    ] {
        let mut storage = store();
        let id = goal(&mut storage, "Hackathon", "park-chat");
        storage.transition_goal(id, "resume").unwrap();
        let conversation = storage.current_conversation().unwrap();
        let state = AppState::new(storage).unwrap();
        let server = MockServer::start().await;
        let instruction = phrase.to_string();
        Mock::given(method("POST")).respond_with(move |request: &Request| {
            let body:Value = serde_json::from_slice(&request.body).unwrap();
            let context:Value = serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            let output = match crate::claude::test::operation(&body).as_str().unwrap() {
                "buddy_skill_plan" => {
                    assert_eq!(context["state"]["focused_goal_id"], id);
                    json!({"kind":"actions","state_version":context["state"]["state_version"],"actions":[{"name":"park_context","input":{"goal":{"goal_id":null,"reference":"active_goal"},"checkpoint":null,"current_app":null},"instruction":instruction}],"clarification":null})
                },
                "buddy_park_checkpoint" => {
                    assert_eq!(context["goal"]["id"], id);
                    assert!(context.get("meaningful_events").is_some());
                    assert!(context.get("personal_memory").is_some());
                    assert!(context.get("window_text").is_none());
                    assert!(context.get("accessibility_text").is_none());
                    json!({"summary":"Scope identified; progress beyond this is unknown","next_action":"Draft requirements","waiting_for":"Team feedback"})
                },
                "buddy_skill_result" => {
                    assert_eq!(context["result"]["results"][0]["data"]["checkpoint"]["next_action"], "Draft requirements");
                    json!({"message":"Hackathon parked. Next: draft requirements. Waiting for team feedback."})
                },
                _ => panic!("Unexpected provider operation"),
            };
            crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(output).to_string()}}]}))
        }).expect(2).mount(&server).await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let reply = structured_chat_actions_with_service(
            &json!({"latest_request":phrase,"conversation":[]}),
            conversation,
            &state,
            0,
            true,
            &service,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(reply.2);
        let inner = state.inner.lock().unwrap();
        let saved = inner.storage.work_context(id).unwrap().unwrap();
        assert_eq!(
            saved.context.next_action.as_deref(),
            Some("Draft requirements")
        );
        assert_eq!(saved.context.waiting_for.as_deref(), Some("Team feedback"));
        assert!(inner.storage.goal().unwrap().is_none());
    }
}

#[tokio::test]
async fn resume_and_inspect_chat_use_exact_context_and_never_other_lanes() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    for (phrase, name, focuses) in [
        ("Верни меня к хакатону", "resume_context", true),
        ("Продолжим Deloitte", "resume_context", true),
        ("Где я остановился?", "inspect_work_context", false),
        (
            "Что я делал здесь последний раз?",
            "inspect_work_context",
            false,
        ),
    ] {
        let mut storage = store();
        let id = goal(&mut storage, "Deloitte Hackathon", "resume-chat");
        let other = goal(&mut storage, "Other project", "other-chat");
        storage
            .park_work_context(
                id,
                crate::work_context::CheckpointDraft {
                    summary: "Outline saved".into(),
                    next_action: "Review requirements".into(),
                    waiting_for: Some("Team feedback".into()),
                },
                None,
            )
            .unwrap();
        storage
            .park_work_context(
                other,
                crate::work_context::CheckpointDraft {
                    summary: "UNRELATED PROJECT CHECKPOINT".into(),
                    next_action: "Other action".into(),
                    waiting_for: None,
                },
                None,
            )
            .unwrap();
        let conversation = storage.current_conversation().unwrap();
        let state = AppState::new(storage).unwrap();
        let server = MockServer::start().await;
        let instruction = phrase.to_string();
        Mock::given(method("POST")).respond_with(move |request: &Request| {
            let body:Value = serde_json::from_slice(&request.body).unwrap();
            let context:Value = serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            let output = if crate::claude::test::operation(&body) == "buddy_skill_plan" {
                let mut input=json!({"goal":{"goal_id":id,"reference":null}});
                if name=="resume_context" {input["set_focus"]=json!(true);}
                json!({"kind":"actions","state_version":context["state"]["state_version"],"actions":[{"name":name,"input":input,"instruction":instruction}],"clarification":null})
            } else {
                assert_eq!(context["result"]["results"][0]["data"]["checkpoint"]["summary"], "Outline saved");
                assert!(!context.to_string().contains("UNRELATED PROJECT CHECKPOINT"));
                assert!(context["personal_memory"].is_object());
                json!({"message":"Outline saved. Next: review requirements. Waiting for team feedback."})
            };
            crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(output).to_string()}}]}))
        }).expect(1).mount(&server).await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let reply = structured_chat_actions_with_service(
            &json!({"latest_request":phrase,"conversation":[]}),
            conversation,
            &state,
            0,
            true,
            &service,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.2, focuses);
        assert!(!reply.0.contains("UNRELATED PROJECT CHECKPOINT"));
        let inner = state.inner.lock().unwrap();
        assert_eq!(
            inner.storage.goal().unwrap().map(|g| g.id),
            if focuses { Some(id) } else { None }
        );
        assert!(inner
            .storage
            .work_context(other)
            .unwrap()
            .unwrap()
            .resumed_at
            .is_none());
    }
}

#[tokio::test]
async fn invalid_or_stale_generated_checkpoint_never_parks_the_goal() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    for stale in [false, true] {
        let mut storage = store();
        let id = goal(&mut storage, "Hackathon", "invalid-park");
        storage.transition_goal(id, "resume").unwrap();
        let conversation = storage.current_conversation().unwrap();
        let state = std::sync::Arc::new(AppState::new(storage).unwrap());
        let captured = state.clone();
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(move |request:&Request| {
            let body:Value=serde_json::from_slice(&request.body).unwrap();
            let context:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            let output=if crate::claude::test::operation(&body)=="buddy_skill_plan" {
                json!({"kind":"actions","state_version":context["state"]["state_version"],"actions":[{"name":"park_context","input":{"goal":{"goal_id":id,"reference":null},"checkpoint":null,"current_app":null},"instruction":"Park this"}],"clarification":null})
            } else if stale {
                captured.inner.lock().unwrap().privacy_revision+=1;
                json!({"summary":"Outline saved","next_action":"Continue","waiting_for":null})
            } else {
                json!({"summary":"Unsafe","next_action":"Continue","waiting_for":null,"raw_accessibility_text":"MUST NOT STORE"})
            };
            crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(output).to_string()}}]}))
        }).mount(&server).await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let reply = structured_chat_actions_with_service(
            &json!({"latest_request":"Park this","conversation":[]}),
            conversation,
            &state,
            0,
            true,
            &service,
        )
        .await;
        assert!(reply.is_err() || !reply.unwrap().unwrap().2);
        let inner = state.inner.lock().unwrap();
        assert_eq!(inner.storage.goal().unwrap().unwrap().id, id);
        assert!(inner
            .storage
            .work_context(id)
            .unwrap()
            .unwrap()
            .parked_at
            .is_none());
    }
}

// Speech candidates replace only the native recognizer in these integration tests.
// Everything after transcription uses the production AgentRuntime/HTTP/skill path.
mod voice_agent {
    use super::*;
    use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};
    fn spoken(text: &str) -> String {
        crate::core_import::select_transcript(
            &json!([{"text":text,"confidence":0.95,"start":0,"end":3}]).to_string(),
        )
        .unwrap()
    }
    fn answer(value: Value) -> ResponseTemplate {
        crate::claude::test::reply(
            json!({"choices":[{"finish_reason":"stop","message":{"content":value.to_string()}}]}),
        )
    }
    async fn mount(server: &MockServer, action: &str, input: Value, decision: &str) {
        let action = action.to_string();
        let decision = decision.to_string();
        Mock::given(method("POST")).respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let serialized = body.to_string();
            assert!(!serialized.contains("RAW_AUDIO_SENTINEL"));
            assert!(!serialized.contains("input_audio"));
            let context: Value = serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            match crate::claude::test::operation(&body).as_str().unwrap() {
                "buddy_skill_plan" => {
                    assert!(["user_message","voice_message"].contains(&context["event_type"].as_str().unwrap()));
                    answer(json!({"situation":"Handle the explicit command","decision":decision,
                        "proposed_actions":if decision=="request_clarification" {json!([])} else {json!([{"name":action,"input":input,"instruction":context["latest_request"]}])},
                        "memory_updates":[],"state_version":context["state"]["state_version"],
                        "clarification":if decision=="request_clarification" {json!("Which goal do you mean?")} else {Value::Null}}))
                },
                "buddy_park_checkpoint" => answer(json!({"summary":"Outline started","next_action":"Write draft","waiting_for":"Review"})),
                "buddy_skill_result" => {
                    assert_eq!(context["result"]["status"],"executed");
                    answer(json!({"message":"Verified backend result"}))
                },
                _ => panic!("Unexpected AI flow; no voice or search pipeline may be used"),
            }
        }).mount(server).await;
    }
    async fn run(
        state: &AppState,
        server: &MockServer,
        text: &str,
        voice: bool,
    ) -> Result<(String, u64, bool), String> {
        let text = if voice {
            spoken(text)
        } else {
            text.to_string()
        };
        let (conversation, revision) = {
            let mut inner = state.inner.lock().unwrap();
            crate::agent_runtime::begin_explicit(
                &mut inner,
                if voice {
                    crate::agent_runtime::EventKind::VoiceMessage
                } else {
                    crate::agent_runtime::EventKind::UserMessage
                },
            );
            (
                inner.storage.current_conversation().unwrap(),
                inner.privacy_revision,
            )
        };
        crate::agent_runtime::handle_explicit_actions(
            &json!({"latest_request":text,"event_type":if voice {"voice_message"}else{"user_message"},"conversation":[],"personal_memory":{}}),
            conversation,state,revision,true,
            &Service{client:&state.client,endpoint:server.uri(),key:"fixture".into(),model:"fixture".into()},
        ).await?.ok_or("Expected a structured agent action".into())
    }
    #[tokio::test]
    async fn mocked_stt_create_goal_and_text_use_the_same_runtime() {
        for voice in [false, true] {
            let state = AppState::new(store()).unwrap();
            let server = MockServer::start().await;
            mount(
                &server,
                "create_goal",
                json!({"title":"Interview","steps":[]}),
                "execute_explicit_user_action",
            )
            .await;
            assert!(
                run(&state, &server, "Create an interview goal", voice)
                    .await
                    .unwrap()
                    .2
            );
            let inner = state.inner.lock().unwrap();
            assert_eq!(
                inner.storage.core_snapshot(None).unwrap().goals[0].title,
                "Interview"
            );
            assert_eq!(
                inner.agent.diagnostics.last_input,
                Some(if voice {
                    crate::agent_runtime::EventKind::VoiceMessage
                } else {
                    crate::agent_runtime::EventKind::UserMessage
                })
            );
        }
    }
    #[tokio::test]
    async fn mocked_stt_create_complete_step_rename_today_and_focus() {
        let mut s = store();
        let id = goal(&mut s, "Interview", "voice-actions");
        let state = AppState::new(s).unwrap();
        for (text, name, input) in [
            (
                "Add a practice step",
                "create_step",
                json!({"goal":{"goal_id":id,"reference":null},"text":"Practice answers"}),
            ),
            (
                "Add a review step",
                "create_step",
                json!({"goal":{"goal_id":id,"reference":null},"text":"Review notes"}),
            ),
            (
                "Complete the first step",
                "complete_step",
                json!({"goal":{"goal_id":id,"reference":null},"step":{"step_id":null,"ordinal":1,"reference":null}}),
            ),
            (
                "Rename this goal to Interview preparation",
                "rename_goal",
                json!({"goal":{"goal_id":id,"reference":null},"title":"Interview preparation"}),
            ),
            (
                "Add this goal to Today",
                "add_goal_to_today",
                json!({"goal":{"goal_id":id,"reference":null}}),
            ),
            (
                "Focus on this goal",
                "set_focus_goal",
                json!({"goal":{"goal_id":id,"reference":null}}),
            ),
        ] {
            let server = MockServer::start().await;
            mount(&server, name, input, "execute_explicit_user_action").await;
            assert!(run(&state, &server, text, true).await.unwrap().2);
        }
        let inner = state.inner.lock().unwrap();
        let snapshot = inner.storage.core_snapshot(None).unwrap();
        let g = &snapshot.goals[0];
        assert_eq!(g.title, "Interview preparation");
        assert_eq!(g.plan.steps.len(), 2);
        assert!(g.plan.steps[0].done);
        assert!(!g.plan.steps[1].done);
        assert!(snapshot.today.contains(&id));
        assert_eq!(snapshot.focused_goal_id, Some(id));
    }
    #[tokio::test]
    async fn mocked_stt_park_resume_and_where_did_i_stop() {
        let mut s = store();
        let id = goal(&mut s, "Hackathon", "voice-park");
        s.transition_goal(id, "resume").unwrap();
        let state = AppState::new(s).unwrap();
        for (text, name, input) in [
            (
                "Сохрани, где я остановился",
                "park_context",
                json!({"goal":{"goal_id":id,"reference":null},"checkpoint":null,"current_app":null}),
            ),
            (
                "Где я остановился?",
                "inspect_work_context",
                json!({"goal":{"goal_id":id,"reference":null}}),
            ),
            (
                "Верни меня к хакатону",
                "resume_context",
                json!({"goal":{"goal_id":id,"reference":null},"set_focus":true}),
            ),
        ] {
            let server = MockServer::start().await;
            mount(&server, name, input, "execute_explicit_user_action").await;
            let reply = run(&state, &server, text, true).await.unwrap();
            assert_eq!(reply.2, name != "inspect_work_context");
            let inner = state.inner.lock().unwrap();
            assert_eq!(
                inner
                    .storage
                    .work_context(id)
                    .unwrap()
                    .unwrap()
                    .context
                    .next_action
                    .as_deref(),
                Some("Write draft")
            );
        }
        assert_eq!(
            state
                .inner
                .lock()
                .unwrap()
                .storage
                .goal()
                .unwrap()
                .unwrap()
                .id,
            id
        );
    }
    #[tokio::test]
    async fn mocked_stt_ambiguous_reference_requests_clarification() {
        let mut s = store();
        goal(&mut s, "Interview", "one");
        goal(&mut s, "Interview", "two");
        let state = AppState::new(s).unwrap();
        let server = MockServer::start().await;
        mount(&server, "create_step", json!({}), "request_clarification").await;
        let reply = run(&state, &server, "Добавь туда шаг", true).await.unwrap();
        assert_eq!(reply.0, "Which goal do you mean?");
        assert!(!reply.2);
        assert!(state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .goals
            .iter()
            .all(|g| g.plan.steps.is_empty()));
    }
    #[tokio::test]
    async fn spoken_goal_delete_shows_transcript_then_needs_separate_confirmation() {
        let mut s = store();
        let id = goal(&mut s, "Interview", "voice-delete");
        let state = AppState::new(s).unwrap();
        let server = MockServer::start().await;
        mount(
            &server,
            "delete_goal",
            json!({"goal":{"goal_id":id,"reference":null}}),
            "execute_explicit_user_action",
        )
        .await;
        let reply = run(&state, &server, "Delete the Interview goal", true)
            .await
            .unwrap();
        assert!(reply
            .0
            .contains("Recognized transcript: Delete the Interview goal"));
        assert!(reply.0.contains("Confirm or Cancel"));
        assert!(reply.0.contains("Delete \"Interview\""));
        assert!(!reply.2);
        assert_eq!(server.received_requests().await.unwrap().len(), 1); // No model-generated preview.
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
            1
        );
        assert!(run(&state, &server, "Confirm", true).await.unwrap().2);
        assert!(state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .goals
            .is_empty());
    }
    #[tokio::test]
    async fn voice_and_text_reject_the_same_invalid_step_without_mutating() {
        for voice in [false, true] {
            let mut s = store();
            let id = goal(&mut s, "Interview", "invalid-step");
            let state = AppState::new(s).unwrap();
            let server = MockServer::start().await;
            mount(&server,"complete_step",json!({"goal":{"goal_id":id,"reference":null},"step":{"step_id":"not-a-real-step","ordinal":null,"reference":null}}),"execute_explicit_user_action").await;
            let reply = run(&state, &server, "Complete that step", voice)
                .await
                .unwrap();
            assert!(reply.0.contains("No changes were made"));
            assert!(!reply.2);
            assert!(state
                .inner
                .lock()
                .unwrap()
                .storage
                .goal_plan(id)
                .unwrap()
                .steps
                .is_empty());
        }
    }
    #[tokio::test]
    async fn voice_today_question_uses_confirmed_backend_history_not_elapsed_time() {
        let mut s = store();
        let id = goal(&mut s, "Interview", "voice-today");
        let c = s.current_conversation().unwrap();
        let snap = crate::skills::snapshot(&s, c).unwrap();
        let plan = crate::skills::Plan {
            kind: "actions".into(),
            state_version: snap.version.clone(),
            actions: vec![crate::skills::Action {
                name: "create_step".into(),
                input: json!({"goal":{"goal_id":id,"reference":null},"text":"Practice answers"}),
                instruction: Some("Add practice step".into()),
            }],
            clarification: None,
        };
        crate::skills::execute(&mut s, &snap, &plan, "Add practice step", true, false).unwrap();
        let mut p = s.goal_plan(id).unwrap();
        p.steps[0].done = true;
        s.core_save_goal("Interview", "Work", p).unwrap();
        let state = AppState::new(s).unwrap();
        let server = MockServer::start().await;
        mount(
            &server,
            "inspect_today",
            json!({}),
            "execute_explicit_user_action",
        )
        .await;
        let reply = run(&state, &server, "Что я сегодня сделал?", true)
            .await
            .unwrap();
        assert!(!reply.2);
        assert!(reply.0.contains("Practice answers"));
        for expected in [
            "- Relevant activity: 0 min",
            "- Observed activity: 0 min",
            "- Manual timer: 0 min",
            "Time alone does not confirm completion.",
        ] {
            assert!(reply.0.contains(expected));
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn spoken_step_delete_cancel_and_stale_confirmation_are_safe() {
        for changed in [false, true] {
            let mut s = store();
            s.core_add_goals(
                vec![crate::core::GoalDraft {
                    title: "Interview".into(),
                    area: "Work".into(),
                    steps: vec!["Practice".into(), "Review".into()],
                }],
                "voice-step-delete",
            )
            .unwrap();
            let id = s.core_snapshot(None).unwrap().goals[0].id;
            let state = AppState::new(s).unwrap();
            let server = MockServer::start().await;
            mount(&server,"delete_step",json!({"goal":{"goal_id":id,"reference":null},"step":{"step_id":null,"ordinal":1,"reference":null}}),"execute_explicit_user_action").await;
            let preview = run(&state, &server, "Delete the first step", true)
                .await
                .unwrap();
            assert!(preview
                .0
                .contains("Recognized transcript: Delete the first step"));
            assert!(preview.0.contains("Practice"));
            assert!(!preview.2);
            if changed {
                state.inner.lock().unwrap().privacy_revision += 1;
                assert!(run(&state, &server, "Confirm", true).await.is_err());
            } else {
                assert!(!run(&state, &server, "Cancel", true).await.unwrap().2);
            }
            let inner = state.inner.lock().unwrap();
            assert_eq!(inner.storage.goal_plan(id).unwrap().steps.len(), 2);
        }
    }
    #[tokio::test]
    async fn today_summary_has_no_yesterday_results_and_sharing_off_blocks_it() {
        let mut s = store();
        let id = goal(&mut s, "Interview", "daily-scope");
        let day = chrono::Local::now().date_naive().to_string();
        let yesterday = (chrono::Local::now().date_naive() - chrono::Duration::days(1)).to_string();
        s.connection.execute("INSERT INTO core_events(goal_id,step_id,title,kind,day,created_at) VALUES(?1,'past','YESTERDAY ONLY','completed',?2,?3)",rusqlite::params![id,yesterday,chrono::Utc::now().to_rfc3339()]).unwrap();
        s.connection.execute("INSERT INTO core_events(goal_id,step_id,title,kind,day,created_at) VALUES(?1,'now','TODAY VERIFIED','completed',?2,?3)",rusqlite::params![id,day,chrono::Utc::now().to_rfc3339()]).unwrap();
        let conversation = s.current_conversation().unwrap();
        let snapshot = crate::skills::snapshot(&s, conversation).unwrap();
        let plan = crate::skills::Plan {
            kind: "actions".into(),
            state_version: snapshot.version.clone(),
            actions: vec![crate::skills::Action {
                name: "inspect_today".into(),
                input: json!({}),
                instruction: None,
            }],
            clarification: None,
        };
        assert!(crate::skills::execute(
            &mut s,
            &snapshot,
            &plan,
            "What did I do today?",
            false,
            false
        )
        .is_err());
        let result = crate::skills::execute(
            &mut s,
            &snapshot,
            &plan,
            "What did I do today?",
            true,
            false,
        )
        .unwrap();
        let receipt = result.fallback_message();
        assert!(receipt.contains("TODAY VERIFIED"));
        assert!(!receipt.contains("YESTERDAY ONLY"));
        assert!(!result.committed_changes());
    }
}

#[tokio::test]
async fn continuity_chat_uses_grounded_backend_summary_without_freeform_regeneration() {
    use wiremock::{matchers::method, Mock, MockServer, Request};
    let mut storage = store();
    let id = goal(&mut storage, "Hackathon fixture", "continuity-chat");
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |request:&Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        assert_eq!(crate::claude::test::operation(&body),"buddy_skill_plan");
        let ctx:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        let plan=json!({"kind":"actions","state_version":ctx["state"]["state_version"],"actions":[{"name":"inspect_continuity","input":{"view":"remaining","goal":{"goal_id":id,"reference":null}},"instruction":null}],"clarification":null});
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":crate::agent_runtime::reasoning::fixture_plan(plan).to_string()}}]}))
    }).expect(1).mount(&server).await;
    let conversation = storage.current_conversation().unwrap();
    let state = AppState::new(storage).unwrap();
    let service = Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    let reply = structured_chat_actions_with_service(
        &json!({"latest_request":"Что осталось по хакатону?","conversation":[]}),
        conversation,
        &state,
        0,
        true,
        &service,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(reply.0.contains("Hackathon fixture"));
    assert!(reply.0.contains("No confirmed completion recorded"));
    assert!(!reply.2);
    assert!(state
        .inner
        .lock()
        .unwrap()
        .storage
        .goal_plan(id)
        .unwrap()
        .steps
        .iter()
        .all(|s| !s.done));
}

#[test]
fn explicit_history_query_stays_in_its_conversation_after_selection_changes() {
    let mut storage = store();
    let first = storage.current_conversation().unwrap();
    let reply = ChatReply {
        message: "First answer".into(),
        ..Default::default()
    };
    storage
        .save_chat_turn("First question", &reply, None)
        .unwrap();
    let second = storage.create_conversation().unwrap();
    storage
        .save_chat_turn("Second question", &reply, None)
        .unwrap();
    assert_eq!(
        history_for(&storage, None).unwrap()[0].text,
        "Second question"
    );
    assert_eq!(
        history_for(&storage, Some(first)).unwrap()[0].text,
        "First question"
    );
    assert_eq!(storage.current_conversation().unwrap(), second);
    storage.delete_conversation(first).unwrap();
    assert!(history_for(&storage, Some(first)).unwrap().is_empty());
}
