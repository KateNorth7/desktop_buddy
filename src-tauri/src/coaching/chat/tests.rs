use super::*;
use crate::core::GoalDraft;
use wiremock::{
    matchers::{body_partial_json, method},
    Mock, MockServer,
};

#[tokio::test]
async fn coaching_response_receives_only_its_bounded_prior_chat_history() {
    let (state, session) = fixture();
    {
        let mut inner = state.inner.lock().unwrap();
        for index in 0..20 {
            append(
                &inner.storage,
                &session,
                "user",
                &format!("own-note-{index}: {}", "é🙂\n".repeat(500)),
            )
            .unwrap();
        }
        inner.storage.create_conversation().unwrap();
        inner
            .storage
            .save_chat_turn(
                "FOREIGN-CHAT-CANARY",
                &ChatReply {
                    message: "FOREIGN-ANSWER-CANARY".into(),
                    ..Default::default()
                },
                None,
            )
            .unwrap();
    }
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request: &wiremock::Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        assert!(!body.to_string().contains("FOREIGN-CHAT-CANARY"));
        let content = if crate::claude::test::operation(&body)=="buddy_coaching_response" {
            assert!(crate::claude::test::system(&body).as_str().unwrap().contains("Sound like a calm colleague, not a coach."));
            assert!(crate::claude::test::system(&body).as_str().unwrap().contains(super::super::voice::VOICE_EXAMPLES));
            let source:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            let history=&source["context"]["history"];
            assert!(history.as_array().unwrap().len()<=12);
            assert!(history.to_string().len()<=6000);
            assert!(history.as_array().unwrap().last().unwrap()["content"].as_str().unwrap().contains("own-note-19"));
            assert!(!history.to_string().contains("CURRENT-ONLY"));
            assert_eq!(source["context"]["latest_message"],"CURRENT-ONLY");
            json!({"message":"Use your last note to choose a heading."})
        } else { json!({"reply":null,"evidence":null,"action":null}) };
        crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":content.to_string()}}]}))
    }).expect(2).mount(&server).await;
    let result = handle(
        &state,
        input(&session, "CURRENT-ONLY", "history-check"),
        Some(&service(&state, &server)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.message, "Use your last note to choose a heading.");
}

fn fixture() -> (AppState, Session) {
    let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    storage
        .core_add_goals(
            vec![
                GoalDraft {
                    title: "Open Cash ISA".into(),
                    area: "Personal".into(),
                    steps: vec!["Read account requirements".into()],
                },
                GoalDraft {
                    title: "UNRELATED HACKATHON GOAL".into(),
                    area: "Work".into(),
                    steps: vec![],
                },
            ],
            "chat-fixture",
        )
        .unwrap();
    let goal = storage
        .core_snapshot(None)
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.title == "Open Cash ISA")
        .unwrap()
        .id;
    let source = storage.current_conversation().unwrap();
    let session = start(
        &storage,
        &Start {
            request_id: "start-chat".into(),
            goal_id: goal,
            step_id: None,
            conversation_id: source,
            expected_target_revision: storage.coaching_target(goal, None).unwrap().revision,
            strategy_id: "tiny_habits".into(),
            strategy_version: 1,
        },
    )
    .unwrap();
    assert_ne!(source, session.conversation_id);
    assert_eq!(session.step, "barrier");
    let state = AppState::new(storage).unwrap();
    state.inner.lock().unwrap().status.mock_ai = false;
    (state, session)
}

fn input<'a>(s: &Session, text: &'a str, key: &'a str) -> Input<'a> {
    Input {
        conversation: s.conversation_id,
        text,
        attachment: "",
        request_id: key,
        session_id: Some(s.id),
        revision: Some(s.revision),
        remember: false,
    }
}
fn current(state: &AppState, s: &Session) -> Session {
    state
        .inner
        .lock()
        .unwrap()
        .storage
        .coaching_session(s.id, s.conversation_id)
        .unwrap()
}
fn service<'a>(state: &'a AppState, server: &MockServer) -> Service<'a> {
    Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "synthetic-test-key".into(),
        model: "fixture-model".into(),
    }
}
async fn provider(server: &MockServer, interpretation: Value) {
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({"output_config":{"format":{"schema":{"description":"buddy_coaching_interpretation"}}}}),
        ))
        .respond_with(
            crate::claude::test::reply(
                json!({"choices":[{"finish_reason":"stop","message":{"content":interpretation.to_string()}}]}),
            ),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"output_config":{"format":{"schema":{"description":"buddy_coaching_response"}}}})))
        .respond_with(crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"message\":\"What would help with this goal?\"}"}}]})))
        .mount(server).await;
}
async fn turn(
    state: &AppState,
    s: &Session,
    text: &str,
    reply: Value,
    action: Value,
    remember: bool,
) -> Session {
    let server = MockServer::start().await;
    provider(
        &server,
        json!({"reply":reply,"evidence":text,"action":action}),
    )
    .await;
    let key = format!("turn-{}", s.revision);
    let mut request = input(s, text, &key);
    request.remember = remember;
    handle(state, request, Some(&service(state, &server)))
        .await
        .unwrap()
        .unwrap();
    for request in server.received_requests().await.unwrap() {
        assert!(!String::from_utf8_lossy(&request.body).contains("UNRELATED HACKATHON GOAL"));
    }
    current(state, s)
}

#[tokio::test]
async fn text_chat_completes_only_micro_action_and_never_receives_unrelated_goals() {
    let (state, mut s) = fixture();
    let before = serde_json::to_value(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .core_snapshot(None)
            .unwrap()
            .goals,
    )
    .unwrap();
    s = turn(
        &state,
        &s,
        "I do not know the first step",
        json!({"type":"barrier","barrier":"unclear_first_step"}),
        json!({"title":"Open the account requirements page","estimated_seconds":20}),
        false,
    )
    .await;
    assert_eq!(s.step, "feasibility");
    s = turn(
        &state,
        &s,
        "8",
        json!({"type":"ease","score":8}),
        Value::Null,
        false,
    )
    .await;
    assert_eq!(s.step, "anchor");
    s = turn(
        &state,
        &s,
        "Skip",
        json!({"type":"anchor","anchor":null}),
        Value::Null,
        false,
    )
    .await;
    s = turn(
        &state,
        &s,
        "Yes, I want to try it",
        json!({"type":"commit","accepted":true}),
        Value::Null,
        false,
    )
    .await;
    s = turn(
        &state,
        &s,
        "I opened the requirements page",
        json!({"type":"report","completed":true,"remember":false}),
        Value::Null,
        false,
    )
    .await;
    assert_eq!(s.status, Status::Completed);
    assert_eq!(s.data["micro_action_completed"], true);
    let inner = state.inner.lock().unwrap();
    assert_eq!(
        before,
        serde_json::to_value(inner.storage.core_snapshot(None).unwrap().goals).unwrap()
    );
    assert!(binding(&inner.storage, s.conversation_id)
        .unwrap()
        .is_some());
    assert_eq!(
        inner
            .storage
            .chat_history_of(s.conversation_id)
            .unwrap()
            .len(),
        12
    );
    assert_eq!(
        inner
            .storage
            .connection
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn blocked_session_stays_goal_bound_until_explicit_exit_and_text_replay_is_exact() {
    let (state, s) = fixture();
    let s = turn(
        &state,
        &s,
        "I am waiting for the bank to approve my documents",
        json!({"type":"barrier","barrier":"external_dependency"}),
        Value::Null,
        false,
    )
    .await;
    assert_eq!(s.status, Status::Blocked);
    state.inner.lock().unwrap().status.mock_ai = true;
    let first = handle(
        &state,
        input(&s, "But I need to do this today", "blocked-chat"),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(first.message.contains("external blocker"));
    let replayed = handle(
        &state,
        input(&s, "But I need to do this today", "blocked-chat"),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.message, replayed.message);
    assert!(
        handle(&state, input(&s, "Different message", "blocked-chat"), None)
            .await
            .is_err()
    );
    let stopped = handle(&state, input(&s, "/exit", "exit-chat"), None)
        .await
        .unwrap()
        .unwrap();
    assert!(stopped.message.contains("Ordinary chat"));
    let replayed = handle(&state, input(&s, "/exit", "exit-chat"), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.message, replayed.message);
    let closed = current(&state, &s);
    assert!(closed.revision > s.revision);
    assert!(
        binding(&state.inner.lock().unwrap().storage, s.conversation_id)
            .unwrap()
            .is_none()
    );
    assert!(
        handle(&state, input(&s, "A stale coaching reply", "late"), None)
            .await
            .is_err()
    );
    let ordinary = Input {
        session_id: None,
        revision: None,
        ..input(&closed, "Ordinary question", "ordinary")
    };
    assert!(handle(&state, ordinary, None).await.unwrap().is_none());
}

#[tokio::test]
async fn replayed_text_has_one_user_and_one_answer_and_does_not_advance_twice() {
    let (state, s) = fixture();
    let server = MockServer::start().await;
    provider(&server,json!({"reply":{"type":"barrier","barrier":"external_dependency"},"evidence":"Waiting for the bank","action":null})).await;
    let first = handle(
        &state,
        input(&s, "Waiting for the bank", "same-turn"),
        Some(&service(&state, &server)),
    )
    .await
    .unwrap()
    .unwrap();
    let second = handle(
        &state,
        input(&s, "Waiting for the bank", "same-turn"),
        Some(&service(&state, &server)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.message, second.message);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .chat_history_of(s.conversation_id)
            .unwrap()
            .len(),
        4
    );
    assert_eq!(current(&state, &s).revision, s.revision + 1);
}

#[tokio::test]
async fn ambiguous_text_clarifies_without_assigning_a_barrier() {
    let (state, s) = fixture();
    let server = MockServer::start().await;
    provider(&server, json!({"reply":null,"evidence":null,"action":null})).await;
    handle(
        &state,
        input(&s, "But I need to do it today", "ambiguous"),
        Some(&service(&state, &server)),
    )
    .await
    .unwrap();
    assert_eq!(current(&state, &s), s);
}

#[tokio::test]
async fn barrier_without_proposal_accepts_an_explicit_text_request_for_a_small_action() {
    let (state, s) = fixture();
    let s = turn(
        &state,
        &s,
        "I do not know where to begin",
        json!({"type":"barrier","barrier":"unclear_first_step"}),
        Value::Null,
        false,
    )
    .await;
    assert_eq!(s.step, "tiny_action");
    let server = MockServer::start().await;
    provider(&server,json!({"reply":null,"evidence":null,"action":{"title":"Open the account requirements","estimated_seconds":10}})).await;
    handle(
        &state,
        input(&s, "Please suggest a small action", "ask-for-action"),
        Some(&service(&state, &server)),
    )
    .await
    .unwrap();
    let next = current(&state, &s);
    assert_eq!(next.step, "feasibility");
    assert_eq!(next.data["action"]["estimated_seconds"], 10);
}

#[test]
fn provider_cannot_confirm_questions_negations_future_plans_or_memory_writes() {
    let (state, s) = fixture();
    let mut execution = s;
    execution.step = "execution".into();
    execution.data["action"] = json!({"title":"Open requirements","estimated_seconds":10});
    for text in [
        "Did I do it?",
        "I have not done it",
        "I will do it",
        "Я не сделал",
        "Я сделаю потом",
    ] {
        let interpretation = Interpretation {
            reply: Some(Reply::Report {
                completed: true,
                remember: false,
            }),
            evidence: Some(text.into()),
            action: None,
        };
        assert!(
            preview(&execution, &interpretation, text, false).is_err(),
            "{text}"
        );
    }
    let malicious = Interpretation {
        reply: Some(Reply::Report {
            completed: true,
            remember: true,
        }),
        evidence: Some("Done".into()),
        action: None,
    };
    assert!(preview(&execution, &malicious, "Done", false).is_err());
    let valid = Interpretation {
        reply: Some(Reply::Report {
            completed: true,
            remember: false,
        }),
        evidence: Some("Done".into()),
        action: None,
    };
    assert_eq!(
        preview(&execution, &valid, "Done", false).unwrap().status,
        Status::Completed
    );
    drop(state);
}

#[tokio::test]
async fn privacy_target_pause_exit_and_forget_during_http_discard_the_pending_transition() {
    for change in ["privacy", "target", "pause", "exit", "forget"] {
        let (state, s) = fixture();
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"reply":{"type":"barrier","barrier":"external_dependency"},"evidence":"Waiting for approval","action":null}).to_string()}}]})).set_delay(std::time::Duration::from_millis(150))).expect(1).mount(&server).await;
        let service = service(&state, &server);
        let request = handle(
            &state,
            input(&s, "Waiting for approval", "pending"),
            Some(&service),
        );
        let mutation = async {
            for _ in 0..100 {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert!(!server.received_requests().await.unwrap().is_empty());
            let mut inner = state.inner.lock().unwrap();
            match change {
                "privacy" => {
                    inner.privacy_revision += 1;
                }
                "target" => {
                    inner
                        .storage
                        .connection
                        .execute(
                            "UPDATE goals SET text='Changed goal' WHERE id=?1",
                            [s.goal_id],
                        )
                        .unwrap();
                }
                "pause" => {
                    control(
                        &inner.storage,
                        s.id,
                        s.conversation_id,
                        s.revision,
                        "interrupt",
                        &Control::Pause,
                    )
                    .unwrap();
                }
                "exit" => {
                    control(
                        &inner.storage,
                        s.id,
                        s.conversation_id,
                        s.revision,
                        "interrupt",
                        &Control::Stop,
                    )
                    .unwrap();
                }
                _ => {
                    inner
                        .storage
                        .forget_coaching(None, Some(s.conversation_id))
                        .unwrap();
                }
            }
        };
        let (result, ()) = tokio::join!(request, mutation);
        assert!(result.is_err(), "{change}");
        if change != "forget" {
            assert!(current(&state, &s).data["barrier"].is_null());
        }
    }
}

#[test]
fn transcript_failure_rolls_back_session_transition_and_binding() {
    let (state, s) = fixture();
    let inner = state.inner.lock().unwrap();
    inner.storage.connection.execute_batch("CREATE TRIGGER fail_chat BEFORE INSERT ON buddy_messages BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(control(
        &inner.storage,
        s.id,
        s.conversation_id,
        s.revision,
        "failed-stop",
        &Control::Stop
    )
    .is_err());
    assert_eq!(
        inner
            .storage
            .coaching_session(s.id, s.conversation_id)
            .unwrap(),
        s
    );
    assert!(binding(&inner.storage, s.conversation_id)
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn failed_answer_write_rolls_back_transition_and_retry_reuses_the_pending_user_message() {
    let (state, s) = fixture();
    state.inner.lock().unwrap().storage.connection.execute_batch("CREATE TRIGGER fail_answer BEFORE INSERT ON buddy_messages WHEN NEW.role='assistant' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let server = MockServer::start().await;
    provider(&server,json!({"reply":{"type":"barrier","barrier":"external_dependency"},"evidence":"Waiting for the bank","action":null})).await;
    let service = service(&state, &server);
    assert!(handle(
        &state,
        input(&s, "Waiting for the bank", "retry-pending"),
        Some(&service)
    )
    .await
    .is_err());
    assert_eq!(current(&state, &s), s);
    {
        let inner = state.inner.lock().unwrap();
        assert_eq!(
            inner
                .storage
                .chat_history_of(s.conversation_id)
                .unwrap()
                .len(),
            3
        );
        inner
            .storage
            .connection
            .execute_batch("DROP TRIGGER fail_answer;")
            .unwrap();
    }
    handle(
        &state,
        input(&s, "Waiting for the bank", "retry-pending"),
        Some(&service),
    )
    .await
    .unwrap();
    assert_eq!(current(&state, &s).status, Status::Blocked);
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .chat_history_of(s.conversation_id)
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test]
async fn sharing_revoked_before_send_and_attachments_do_not_reach_the_provider() {
    let (state, s) = fixture();
    let server = MockServer::start().await;
    let service = service(&state, &server);
    let request = Input {
        attachment: "Private attachment",
        ..input(&s, "Help", "attachment")
    };
    assert!(handle(&state, request, Some(&service)).await.is_err());
    {
        let inner = state.inner.lock().unwrap();
        let mut preferences = inner.storage.ai_preferences().unwrap();
        preferences.share_goal_context = false;
        inner
            .storage
            .write_setting("ai_assistance_preferences", &preferences)
            .unwrap();
    }
    let result = handle(&state, input(&s, "Help", "sharing-off"), Some(&service))
        .await
        .unwrap()
        .unwrap();
    assert!(result.message.contains("sharing settings changed"));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(current(&state, &s), s);
}

#[tokio::test]
async fn text_completion_saves_a_memory_only_with_the_explicit_checkbox() {
    for remember in [false, true] {
        let (state, mut s) = fixture();
        {
            let inner = state.inner.lock().unwrap();
            inner.storage.connection.execute("UPDATE memory_controls SET enabled=1,remember_work_context=1,learn_from_feedback=1 WHERE id=1",[]).unwrap();
            let strategy = strategy::find(&s.strategy_id, s.strategy_version).unwrap();
            strategy
                .handler
                .reply(
                    &mut s,
                    &json!(Reply::Barrier {
                        barrier: Barrier::Overwhelm
                    }),
                    strategy,
                )
                .unwrap();
            strategy
                .handler
                .propose(
                    &mut s,
                    &json!({"title":"Open the account requirements","estimated_seconds":10}),
                    strategy,
                )
                .unwrap();
            for reply in [
                Reply::Ease { score: 8 },
                Reply::Anchor { anchor: None },
                Reply::Commit { accepted: true },
            ] {
                strategy
                    .handler
                    .reply(&mut s, &json!(reply), strategy)
                    .unwrap();
            }
            s.revision += 5;
            inner
                .storage
                .save_coaching(&s, "execution-fixture", "synthetic")
                .unwrap();
        }
        let s = turn(
            &state,
            &s,
            "I opened the account requirements",
            json!({"type":"report","completed":true,"remember":false}),
            Value::Null,
            remember,
        )
        .await;
        assert_eq!(s.status, Status::Completed);
        let inner = state.inner.lock().unwrap();
        let count: i64 = inner
            .storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source_key=?1",
                [format!("coaching-micro-action-{}", s.id)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, i64::from(remember));
    }
}

#[test]
fn migration_binds_existing_ended_sessions_and_forgetting_removes_turn_metadata() {
    let (state, s) = fixture();
    let inner = state.inner.lock().unwrap();
    let ended = inner
        .storage
        .reply_coaching(
            s.id,
            s.conversation_id,
            s.revision,
            "legacy-stop",
            &Control::Stop,
        )
        .unwrap();
    inner.storage.connection.execute_batch("DROP TABLE coaching_chat_turns; DROP TABLE coaching_chat_starts; DROP TABLE coaching_chat_bindings; DELETE FROM coaching_migrations WHERE version=2;").unwrap();
    inner.storage.initialize_coaching().unwrap();
    assert_eq!(
        binding(&inner.storage, s.conversation_id).unwrap(),
        Some(ended)
    );
    inner
        .storage
        .forget_coaching(None, Some(s.conversation_id))
        .unwrap();
    assert!(binding(&inner.storage, s.conversation_id)
        .unwrap()
        .is_none());
    assert_eq!(
        inner
            .storage
            .chat_history_of(s.conversation_id)
            .unwrap()
            .len(),
        2
    );
}
