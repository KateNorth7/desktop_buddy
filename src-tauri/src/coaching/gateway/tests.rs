use super::*;
use crate::core::GoalDraft;
use std::time::Duration;
use wiremock::{
    matchers::{header, method},
    Mock, MockServer, ResponseTemplate,
};

fn fixture() -> (AppState, Session) {
    let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    storage
        .core_add_goals(
            vec![GoalDraft {
                title: "Draft project proposal".into(),
                area: "Work".into(),
                steps: vec!["Draft introduction".into()],
            }],
            "gateway-fixture",
        )
        .unwrap();
    let goal = storage.core_snapshot(None).unwrap().goals[0].id;
    let conversation = storage.current_conversation().unwrap();
    let session = storage
        .start_coaching(&Start {
            request_id: "start".into(),
            goal_id: goal,
            step_id: Some("step-0".into()),
            conversation_id: conversation,
            expected_target_revision: storage
                .coaching_target(goal, Some("step-0"))
                .unwrap()
                .revision,
            strategy_id: "tiny_habits".into(),
            strategy_version: 1,
        })
        .unwrap();
    let session = respond(&storage, &session, Reply::Ready { ready: true });
    let session = respond(
        &storage,
        &session,
        Reply::Barrier {
            barrier: Barrier::Overwhelm,
        },
    );
    let state = AppState::new(storage).unwrap();
    state.inner.lock().unwrap().status.mock_ai = false;
    (state, session)
}

fn respond(storage: &Storage, s: &Session, reply: Reply) -> Session {
    storage
        .reply_coaching(
            s.id,
            s.conversation_id,
            s.revision,
            &format!("reply-{}", s.revision),
            &Control::Reply {
                reply: reply.into(),
            },
        )
        .unwrap()
}

fn response(action: Value) -> Value {
    json!({"choices":[{"finish_reason":"stop","message":{"content":action.to_string()}}]})
}

fn service<'a>(state: &'a AppState, server: &MockServer) -> Service<'a> {
    Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "synthetic-test-key".into(),
        model: "fixture-model".into(),
    }
}

#[tokio::test]
async fn real_http_proposal_completes_only_the_confirmed_micro_action() {
    let (state, initial) = fixture();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("x-api-key", "synthetic-test-key"))
        .respond_with(crate::claude::test::reply(response(
            json!({"title":"Open the proposal and type its heading","estimated_seconds":20}),
        )))
        .expect(1)
        .mount(&server)
        .await;
    let service = service(&state, &server);
    let next = generate(
        &state,
        Some(&service),
        initial.id,
        initial.conversation_id,
        initial.revision,
        "proposal",
    )
    .await
    .unwrap();
    assert_eq!(next.session.step, "feasibility");
    assert_eq!(next.session.data["action"]["estimated_seconds"], 20);
    {
        let inner = state.inner.lock().unwrap();
        let before =
            serde_json::to_value(inner.storage.core_snapshot(None).unwrap().goals).unwrap();
        let session = respond(&inner.storage, &next.session, Reply::Ease { score: 8 });
        let session = respond(&inner.storage, &session, Reply::Anchor { anchor: None });
        let session = respond(&inner.storage, &session, Reply::Commit { accepted: true });
        let session = respond(
            &inner.storage,
            &session,
            Reply::Report {
                completed: true,
                remember: false,
            },
        );
        assert_eq!(session.status, Status::Completed);
        assert_eq!(
            serde_json::to_value(inner.storage.core_snapshot(None).unwrap().goals).unwrap(),
            before
        );
    }
    let requests = server.received_requests().await.unwrap();
    let payload: Value = requests[0].body_json().unwrap();
    assert_eq!(
        crate::claude::test::operation(&payload),
        "buddy_coaching_proposal"
    );
    assert!(payload.to_string().contains("Draft project proposal"));
    let system = crate::claude::test::system(&payload).as_str().unwrap();
    assert!(system.contains("Sound like a calm colleague, not a coach."));
    assert!(system.contains("Write the title in the language of the goal text"));
    assert!(!system.contains(super::super::voice::VOICE_EXAMPLES));
    assert!(!payload.to_string().contains("synthetic-test-key"));
}

#[tokio::test]
async fn malformed_proposals_have_one_repair_and_preserve_the_session() {
    let (state, initial) = fixture();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(crate::claude::test::reply(response(
            json!({"title":"Pretend complete","estimated_seconds":20,"next_state":"completed"}),
        )))
        .expect(2)
        .mount(&server)
        .await;
    let service = service(&state, &server);
    assert!(generate(
        &state,
        Some(&service),
        initial.id,
        initial.conversation_id,
        initial.revision,
        "proposal"
    )
    .await
    .is_err());
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .coaching_session(initial.id, initial.conversation_id)
            .unwrap(),
        initial
    );
    let requests = server.received_requests().await.unwrap();
    let payload: Value = requests[1].body_json().unwrap();
    let system = crate::claude::test::system(&payload).as_str().unwrap();
    assert!(system.contains("last proposal failed"));
    assert!(!system.contains("Copy instruction"));
}

#[tokio::test]
async fn disabled_sharing_never_sends_a_provider_request() {
    for disable_ai in [false, true] {
        let (state, initial) = fixture();
        let server = MockServer::start().await;
        {
            let inner = state.inner.lock().unwrap();
            let mut prefs = inner.storage.ai_preferences().unwrap();
            if disable_ai {
                prefs.enabled = false;
            } else {
                prefs.share_goal_context = false;
            }
            inner
                .storage
                .write_setting("ai_assistance_preferences", &prefs)
                .unwrap();
        }
        let service = service(&state, &server);
        assert!(generate(
            &state,
            Some(&service),
            initial.id,
            initial.conversation_id,
            initial.revision,
            "proposal"
        )
        .await
        .is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn in_flight_privacy_target_and_pause_changes_discard_provider_output() {
    for change in ["privacy", "target", "pause", "forget"] {
        let (state, initial) = fixture();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                crate::claude::test::reply(response(json!({"title":"Open the proposal","estimated_seconds":10})))
                    .set_delay(Duration::from_millis(250)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let service = service(&state, &server);
        let request = generate(
            &state,
            Some(&service),
            initial.id,
            initial.conversation_id,
            initial.revision,
            "proposal",
        );
        let mutation = async {
            // Mutate only after the HTTP request was received and before the delayed answer.
            for _ in 0..100 {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
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
                            "UPDATE goals SET text='Updated proposal' WHERE id=?1",
                            [initial.goal_id],
                        )
                        .unwrap();
                }
                "pause" => {
                    inner
                        .storage
                        .reply_coaching(
                            initial.id,
                            initial.conversation_id,
                            initial.revision,
                            "pause",
                            &Control::Pause,
                        )
                        .unwrap();
                }
                _ => {
                    inner
                        .storage
                        .forget_coaching(None, Some(initial.conversation_id))
                        .unwrap();
                }
            }
        };
        let (result, ()) = tokio::join!(request, mutation);
        assert!(
            result.is_err(),
            "in-flight {change} must reject the proposal"
        );
        let inner = state.inner.lock().unwrap();
        if change != "forget" {
            let session = inner
                .storage
                .coaching_session(initial.id, initial.conversation_id)
                .unwrap();
            assert!(session.data["action"].is_null());
            assert_eq!(
                session.status,
                if change == "pause" {
                    Status::Paused
                } else {
                    Status::Active
                }
            );
        }
    }
}

#[tokio::test]
async fn provider_errors_and_stale_retries_do_not_consume_attempts() {
    let (state, initial) = fixture();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    let service = service(&state, &server);
    assert!(generate(
        &state,
        Some(&service),
        initial.id,
        initial.conversation_id,
        initial.revision,
        "failed"
    )
    .await
    .unwrap_err()
    .contains("401"));
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .coaching_session(initial.id, initial.conversation_id)
            .unwrap(),
        initial
    );
    assert!(generate(
        &state,
        Some(&service),
        initial.id,
        initial.conversation_id,
        initial.revision - 1,
        "stale"
    )
    .await
    .is_err());
}

#[test]
fn coaching_only_chats_survive_pruning_and_new_chat_does_not_reuse_them() {
    let (state, session) = fixture();
    let inner = state.inner.lock().unwrap();
    assert!(inner.storage.start_fresh_conversation().unwrap());
    let other = inner.storage.current_conversation().unwrap();
    assert_ne!(other, session.conversation_id);
    inner.storage.prune_empty_conversations(other).unwrap();
    assert_eq!(
        inner
            .storage
            .coaching_session(session.id, session.conversation_id)
            .unwrap(),
        session
    );
    assert!(view(&inner.storage, session).unwrap().fresh);
}

#[tokio::test]
async fn privacy_revoked_during_an_invalid_reply_prevents_a_repair_request() {
    let (state, initial) = fixture();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            crate::claude::test::reply(response(json!({"title":"Open proposal","estimated_seconds":1})))
                .set_delay(Duration::from_millis(250)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let service = service(&state, &server);
    let request = generate(
        &state,
        Some(&service),
        initial.id,
        initial.conversation_id,
        initial.revision,
        "proposal",
    );
    let revoke = async {
        for _ in 0..100 {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!server.received_requests().await.unwrap().is_empty());
        state.inner.lock().unwrap().privacy_revision += 1;
    };
    let (result, ()) = tokio::join!(request, revoke);
    assert!(result.unwrap_err().contains("privacy changed"));
    assert_eq!(
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .coaching_session(initial.id, initial.conversation_id)
            .unwrap(),
        initial
    );
}

#[test]
fn proposals_reject_unsafe_text_and_unapproved_state_fields() {
    for invalid in [
        json!({"title":"Open https://example.com","estimated_seconds":10}),
        json!({"title":"Use password: demo","estimated_seconds":10}),
        json!({"title":"Open proposal","estimated_seconds":61}),
        json!({"title":"Open proposal","estimated_seconds":10,"completed":true}),
    ] {
        assert!(parse(&response(invalid)).is_err());
    }
}
