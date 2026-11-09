//! Synthetic end-to-end regression through production planner, validators, storage and research.
//! Transcript and HTTP fixtures do not claim live microphone, Accessibility or provider coverage.
use super::*;
use crate::{
    agent_runtime,
    companion::{
        self,
        engine::{Detection, Intervention, Kind},
    },
    continuity,
    memory::{MemoryInput, MemoryQuery, MemoryType, Scope},
    storage::Storage,
};
use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};
fn envelope(value: Value) -> ResponseTemplate {
    crate::claude::test::reply(
        json!({"model":"fixture-reported-model","choices":[{"finish_reason":"stop","message":{"content":value.to_string()}}]}),
    )
}
fn target(id: i64) -> Value {
    json!({"goal_id":id,"reference":null})
}
fn action(name: &str, input: Value, instruction: &str) -> Value {
    json!({"name":name,"input":input,"instruction":instruction})
}
fn plan(source: &Value) -> Value {
    let latest = source["latest_request"].as_str().unwrap();
    let goals = source["state"]["goals"].as_array().unwrap();
    let find = |title: &str| {
        goals.iter().find(|g| g["title"] == title).unwrap()["goal_id"]
            .as_i64()
            .unwrap()
    };
    let actions = match latest {
        "Today focus on finishing the technical hackathon application" => vec![
            action(
                "create_goal",
                json!({"title":"Hackathon","steps":["Draft technical application","Submit technical application","Review final checklist"]}),
                latest,
            ),
            action(
                "set_focus_goal",
                json!({"goal":{"goal_id":null,"reference":"previous_action"}}),
                latest,
            ),
        ],
        "Park this, I need to reply to a client" => vec![action(
            "park_context",
            json!({"goal":target(find("Hackathon")),"checkpoint":null,"current_app":null}),
            latest,
        )],
        "Create a Client goal and focus it" => vec![
            action(
                "create_goal",
                json!({"title":"Client","steps":["Send client reply","Review response"]}),
                latest,
            ),
            action(
                "set_focus_goal",
                json!({"goal":{"goal_id":null,"reference":"previous_action"}}),
                latest,
            ),
        ],
        "Park the client work while waiting for review" => vec![action(
            "park_context",
            json!({"goal":target(find("Client")),"checkpoint":null,"current_app":null}),
            latest,
        )],
        "Return me to the hackathon" => vec![action(
            "resume_context",
            json!({"goal":target(find("Hackathon")),"set_focus":true}),
            latest,
        )],
        "Mark the first hackathon step complete" => vec![action(
            "complete_step",
            json!({"goal":target(find("Hackathon")),"step":{"step_id":null,"ordinal":1,"reference":null}}),
            latest,
        )],
        "What did I do today?" => vec![action(
            "inspect_continuity",
            json!({"view":"end_of_day","goal":null}),
            latest,
        )],
        _ => panic!("Unexpected fixture message: {latest}"),
    };
    json!({"situation":"Explicit fixture instruction","decision":"execute_explicit_user_action","proposed_actions":actions,"memory_updates":[],"state_version":source["state"]["state_version"],"clarification":null})
}
async fn send(state: &AppState, service: &Service<'_>, text: &str, voice: bool) -> String {
    let (conversation, revision) = {
        let mut inner = state.inner.lock().unwrap();
        agent_runtime::begin_explicit(
            &mut inner,
            if voice {
                agent_runtime::EventKind::VoiceMessage
            } else {
                agent_runtime::EventKind::UserMessage
            },
        );
        (
            inner.storage.current_conversation().unwrap(),
            inner.privacy_revision,
        )
    };
    agent_runtime::handle_explicit_actions(&json!({"latest_request":text,"event_type":if voice {"voice_message"}else{"user_message"},"conversation":[]}),conversation,state,revision,true,service).await.unwrap().unwrap().0
}
fn id(state: &AppState, title: &str) -> i64 {
    state
        .inner
        .lock()
        .unwrap()
        .storage
        .core_snapshot(None)
        .unwrap()
        .goals
        .iter()
        .find(|g| g.title == title)
        .unwrap()
        .id
}
fn confirm_receipt(state: &AppState, goal: i64, step: &str) {
    let mut inner = state.inner.lock().unwrap();
    let plan = inner.storage.goal_plan(goal).unwrap();
    let visible = "Message sent successfully. Client reply delivered.";
    let detection = Detection {
        kind: "completion".into(),
        text: None,
        step_id: Some(step.into()),
        confidence: 0.96,
        evidence: Some("Client reply delivered.".into()),
    }
    .validate(&plan)
    .unwrap()
    .validate_evidence(visible)
    .unwrap();
    assert!(!inner.storage.goal_plan(goal).unwrap().steps[0].done);
    let prompt = Intervention {
        id: "synthetic-confirmation".into(),
        kind: Kind::Completion,
        text: "Mark this reply complete?".into(),
        confidence: detection.confidence,
        goal_id: Some(goal),
        plan_revision: Some(plan.revision),
        step_id: detection.step_id,
        expires_at: chrono::Utc::now().timestamp() + 60,
    };
    companion::confirm_completion(&mut inner.storage, &prompt).unwrap();
    assert!(inner.storage.goal_plan(goal).unwrap().steps[0].done);
}
#[tokio::test]
async fn hackathon_hero_two_lanes_voice_park_receipt_resume_research_summary_restart() {
    let path = std::env::temp_dir().join(format!(
        "buddy-hero-{}.sqlite",
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let state = AppState::new(Storage::open(&path).unwrap()).unwrap();
    state.inner.lock().unwrap().status.mock_ai = false;
    let claude = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request:&Request|{
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        if body.get("tools").is_some() {
            assert_eq!(body["messages"][0]["content"],"technical documentation submission checklist");
            assert!(body.get("memories").is_none() && body.get("goal").is_none());
            return crate::claude::test::search_reply(json!([{"title":"Documentation","url":"https://docs.rs/","content":"Describe the architecture, tests and remaining limitations."}]));
        }
        let source:Value=serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        assert!(body.get("audio").is_none());assert!(!body.to_string().contains("RAW_ACCESSIBILITY_CANARY"));
        match crate::claude::test::operation(&body).as_str().unwrap() {
            "buddy_skill_plan"=>envelope(plan(&source)),
            "buddy_park_checkpoint"=> {
                let client=source.to_string().contains("Client");
                envelope(if client {json!({"summary":"Client reply sent; review remains open.","next_action":"Review the client response","waiting_for":"Client review"})}else{json!({"summary":"Technical application draft remains unfinished.","next_action":"Draft technical application","waiting_for":null})})
            },
            "buddy_skill_result"=> {
                if source["result"]["results"][0]["skill"]=="resume_context" {
                    let context=source.to_string();
                    assert!(context.contains("Technical application draft remains unfinished"));
                    assert!(context.contains("Prefer official documentation"));
                    assert!(!context.contains("CLIENT_PRIVATE_CANARY"));
                }
                envelope(json!({"message":"The validated fixture result is available."}))
            },
            "buddy_contextual_search_intent"=>{
                assert!(!source.to_string().contains("CLIENT_PRIVATE_CANARY"));
                envelope(json!({"query":"technical documentation submission checklist"}))
            },
            "buddy_research_evidence"=>envelope(json!({"sources":[{"source_index":0,"applicability":"applicable","reason":"Fixture documentation","excerpt_ids":[0]}]})),
            "buddy_contextual_research_result"=>envelope(json!({"summary":"Review the technical documentation checklist.","next_action":"Draft the architecture section using the checklist.","source_indices":[0]})),
            other=>panic!("Unexpected inference operation: {other}"),
        }
    }).mount(&claude).await;
    {
        let service = Service {
            client: &state.client,
            endpoint: claude.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        send(
            &state,
            &service,
            "Today focus on finishing the technical hackathon application",
            true,
        )
        .await;
        let hackathon = id(&state, "Hackathon");
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
            hackathon
        );
        send(
            &state,
            &service,
            "Park this, I need to reply to a client",
            false,
        )
        .await;
        assert_eq!(
            state
                .inner
                .lock()
                .unwrap()
                .storage
                .work_context(hackathon)
                .unwrap()
                .unwrap()
                .context
                .next_action
                .as_deref(),
            Some("Draft technical application")
        );
        send(&state, &service, "Create a Client goal and focus it", false).await;
        let client = id(&state, "Client");
        confirm_receipt(&state, client, "step-0");
        send(
            &state,
            &service,
            "Park the client work while waiting for review",
            false,
        )
        .await;
        {
            let inner = state.inner.lock().unwrap();
            for (goal, kind, text) in [
                (
                    hackathon,
                    MemoryType::ExplicitPreference,
                    "Prefer official documentation",
                ),
                (client, MemoryType::ExplicitFact, "CLIENT_PRIVATE_CANARY"),
            ] {
                inner
                    .storage
                    .create_memory(MemoryInput {
                        scope: Scope::goal(goal),
                        memory_type: kind,
                        content: text.into(),
                    })
                    .unwrap();
            }
        }
        send(&state, &service, "Return me to the hackathon", false).await;
        assert!(claude.received_requests().await.unwrap().iter().all(|r| {
            serde_json::from_slice::<Value>(&r.body)
                .unwrap()
                .get("tools")
                .is_none()
        }));
        let research_id = {
            let mut inner = state.inner.lock().unwrap();
            let offer =
                crate::research::stuck_offer(&inner, Some(hackathon), Some("step-0")).unwrap();
            assert_eq!(
                inner
                    .storage
                    .connection
                    .query_row("SELECT COUNT(*) FROM contextual_research", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );

            crate::research::respond_offer(&mut inner, &offer, true)
                .unwrap()
                .unwrap()
        };
        let research = crate::research::run_with_service(&state, hackathon, research_id, &service)
            .await
            .unwrap();
        assert_eq!(research.sources.len(), 1);
        state
            .inner
            .lock()
            .unwrap()
            .storage
            .research_feedback(hackathon, research_id, true)
            .unwrap();
        send(
            &state,
            &service,
            "Mark the first hackathon step complete",
            false,
        )
        .await;
        let summary = send(&state, &service, "What did I do today?", false).await;
        assert!(summary.contains("Hackathon") && summary.contains("Client"));
        assert!(
            summary.contains("Draft technical application")
                && summary.contains("Send client reply")
        );
        assert!(!summary.contains("CLIENT_PRIVATE_CANARY"));
        println!("Hero fixture grounded summary:\n{summary}");
        let calls = claude.received_requests().await.unwrap();
        assert_eq!(calls.len(), 13); // 7 plans + 2 checkpoints + 3 research calls (including evidence) + 1 web search; action receipts are local.
        let panel = view(&state.inner.lock().unwrap()).unwrap();
        assert_eq!(panel["last_skill"], "inspect_continuity");
        assert_eq!(panel["last_inference"]["status"], "success");
    }
    let hackathon = id(&state, "Hackathon");
    let client = id(&state, "Client");
    drop(state);
    let reopened = Storage::open(&path).unwrap();
    let context = reopened.work_context(hackathon).unwrap().unwrap();
    assert!(context.resumed_at.is_some());
    assert_eq!(reopened.context_research(context.id).unwrap().len(), 1);
    assert_eq!(
        reopened
            .work_context(client)
            .unwrap()
            .unwrap()
            .context
            .waiting_for
            .as_deref(),
        Some("Client review")
    );
    assert_eq!(
        reopened
            .list_memories(MemoryQuery {
                scope: Some(Scope::goal(hackathon)),
                ..Default::default()
            })
            .unwrap()
            .iter()
            .filter(|m| m.memory_type == MemoryType::ExplicitPreference)
            .count(),
        1
    );
    let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
    let report = reopened
        .continuity_report(continuity::View::Yesterday, None, tomorrow)
        .unwrap();
    assert_eq!(report.lanes.iter().flat_map(|l| &l.completed).count(), 2);
    assert!(report
        .lanes
        .iter()
        .flat_map(|l| &l.completed)
        .any(|c| c.source == "ai_suggested_user_confirmed"));
    assert!(report
        .lanes
        .iter()
        .flat_map(|l| &l.completed)
        .any(|c| c.source == "manual"));
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}
