use super::*;
use crate::{
    core::GoalDraft,
    memory::{EventKind as MemoryEvent, MemoryInput, MemoryType, Scope},
    models::ActivitySnapshot,
};
use reasoning::MemoryUpdate;
use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};
fn storage() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}
fn add(s: &mut Storage, title: &str, batch: &str) -> i64 {
    s.core_add_goals(
        vec![GoalDraft {
            title: title.into(),
            area: "Work".into(),
            steps: vec!["Write outline".into(), "Review draft".into()],
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
fn state() -> (AppState, i64) {
    let mut s = storage();
    let id = add(&mut s, "Hackathon", "runtime-test");
    s.core_today(id, true, &Local::now().date_naive().to_string())
        .unwrap();
    s.transition_goal(id, "resume").unwrap();
    let mut settings = s.user_settings().unwrap();
    settings.onboarding.completed = true;
    s.write_setting("user_settings", &settings).unwrap();
    let state = AppState::new(s).unwrap();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.status.tracking = true;
        crate::tracking::allow_notifications(&mut inner);
        inner.companion.view.preferences.screen_task_detection = true;
        inner.status.mock_ai = false;
        inner.status.anthropic_configured = true;
        inner.companion.view.preferences.quiet_start_minute = 0;
        inner.companion.view.preferences.quiet_end_minute = 0;
        inner.activity_state.state = crate::tracking::ActivityState::Focused;
        inner.buddy.foreground = Some(ActivitySnapshot {
            process_name: "Editor".into(),
            window_title: "PRIVATE RAW WINDOW".into(),
            idle_seconds: 10,
            ..Default::default()
        });
    }
    (state, id)
}
fn event(kind: EventKind, id: i64) -> Event {
    Event::new(kind, Some(id), Utc::now().timestamp() - 10)
}
fn response(value: Value) -> ResponseTemplate {
    crate::claude::test::reply(
        json!({"choices":[{"finish_reason":"stop","message":{"content":value.to_string()}}]}),
    )
}
fn reasoning(source: &Value, decision: &str) -> Value {
    json!({"situation":"Useful next step","decision":decision,"proposed_actions":[],"memory_updates":[],"state_version":source["state_version"],"clarification":null})
}
fn service<'a>(state: &'a AppState, server: &MockServer) -> Service<'a> {
    Service {
        client: &state.client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    }
}
async fn mount_silent(server: &MockServer, n: u64) {
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(
                crate::claude::test::operation(&body),
                "buddy_agent_reasoning"
            );
            let source: Value =
                serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            response(reasoning(&source, "stay_silent"))
        })
        .expect(n)
        .mount(server)
        .await;
}
#[tokio::test]
async fn insignificant_collector_ticks_do_not_call_ai() {
    let (state, _) = state();
    let server = MockServer::start().await;
    mount_silent(&server, 0).await;
    {
        let mut inner = state.inner.lock().unwrap();
        let now = Utc::now().timestamp();
        observe::observe(&mut inner, now).unwrap();
        inner.agent.pending.clear();
        for offset in 1..20 {
            observe::observe(&mut inner, now + offset).unwrap();
        }
        assert!(inner.agent.pending.is_empty());
    }
    assert!(run_pending_with_service(&state, &service(&state, &server))
        .await
        .unwrap()
        .is_none());
}
#[tokio::test]
async fn meaningful_event_calls_ai_and_stay_silent_has_no_action_or_memory_effect() {
    let (state, id) = state();
    let server = MockServer::start().await;
    mount_silent(&server, 1).await;
    let before = state
        .inner
        .lock()
        .unwrap()
        .storage
        .goal_plan(id)
        .unwrap()
        .revision;
    state
        .inner
        .lock()
        .unwrap()
        .agent
        .enqueue(event(EventKind::StepChanged, id));
    assert!(run_pending_with_service(&state, &service(&state, &server))
        .await
        .unwrap()
        .is_none());
    let inner = state.inner.lock().unwrap();
    assert_eq!(inner.agent.diagnostics.ai_calls, 1);
    assert_eq!(
        inner.agent.diagnostics.last_decision,
        Some(Decision::StaySilent)
    );
    assert_eq!(inner.storage.goal_plan(id).unwrap().revision, before);
}
#[tokio::test]
async fn dnd_and_each_privacy_gate_suppress_ai() {
    let server = MockServer::start().await;
    mount_silent(&server, 0).await;
    for gate in 0..9 {
        let (state, id) = state();
        {
            let mut inner = state.inner.lock().unwrap();
            match gate {
                0 => inner.status.dnd = true,
                1 => inner.status.tracking = false,
                2 => inner.companion.view.preferences.paused = true,
                3 => inner.buddy.foreground.as_mut().unwrap().media_playing = true,
                4 => {
                    inner.buddy.foreground.as_mut().unwrap().window_title =
                        "password settings".into()
                }
                5 => inner.status.tracking_error = Some("Permission missing".into()),
                6 => {
                    let mut p = inner.storage.ai_preferences().unwrap();
                    p.share_goal_context = false;
                    inner
                        .storage
                        .write_setting("ai_assistance_preferences", &p)
                        .unwrap();
                }
                7 => {
                    let mut p = inner.storage.ai_preferences().unwrap();
                    p.enabled = false;
                    inner
                        .storage
                        .write_setting("ai_assistance_preferences", &p)
                        .unwrap();
                }
                _ => {
                    inner.status.ai_enabled = false;
                    inner.companion.view.preferences.screen_task_detection = false;
                }
            }
            inner.agent.enqueue(event(EventKind::PossibleStuck, id));
        }
        assert!(
            run_pending_with_service(&state, &service(&state, &server))
                .await
                .unwrap()
                .is_none(),
            "gate {gate}"
        );
    }
}
#[test]
fn debounce_cooldown_and_bounded_queue_coalesce_events() {
    let mut runtime = AgentRuntime::load(&storage()).unwrap();
    runtime.enqueue(Event::new(EventKind::GoalChanged, Some(1), 1000));
    assert!(runtime.ready(1004, true).is_none());
    assert!(runtime.ready(1005, true).is_some());
    runtime.saved.last_attempt = 1005;
    for time in 1010..1100 {
        runtime.enqueue(Event::new(EventKind::StepChanged, Some(1), time));
    }
    assert_eq!(runtime.pending.len(), 1);
    assert!(runtime.ready(1304, true).is_none());
    assert!(runtime.ready(1305, true).is_some());
    for id in 1..100 {
        runtime.enqueue(Event::new(EventKind::GoalChanged, Some(id), 1400));
    }
    assert_eq!(runtime.pending.len(), MAX_PENDING);
    assert!(runtime.ready(1800, true).is_none());
}
#[tokio::test]
async fn scoped_retrieval_precedes_reasoning_and_excludes_other_projects_and_screen_text() {
    let (state, id) = state();
    let server = MockServer::start().await;
    {
        let mut inner = state.inner.lock().unwrap();
        let other = add(&mut inner.storage, "Other", "other-test");
        for (goal, content) in [
            (id, "Relevant confirmed fact"),
            (other, "UNRELATED SECRET PROJECT"),
        ] {
            inner
                .storage
                .create_memory(MemoryInput {
                    memory_type: MemoryType::ExplicitFact,
                    scope: Scope::goal(goal),
                    content: content.into(),
                })
                .unwrap();
        }
        inner.agent.enqueue(event(EventKind::GoalChanged, id));
    }
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let source: Value =
                serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            let text = source.to_string();
            assert!(text.contains("Relevant confirmed fact"));
            assert!(!text.contains("UNRELATED"));
            assert!(!text.contains("PRIVATE RAW WINDOW"));
            response(reasoning(&source, "stay_silent"))
        })
        .expect(1)
        .mount(&server)
        .await;
    run_pending_with_service(&state, &service(&state, &server))
        .await
        .unwrap();
}
#[test]
fn explicit_user_action_uses_the_registry_and_background_cannot_execute_it() {
    let (state, id) = state();
    let mut inner = state.inner.lock().unwrap();
    inner.status.dnd = true;
    let conversation = inner.storage.current_conversation().unwrap();
    let snapshot = skills::snapshot(&inner.storage, conversation).unwrap();
    let plan = json!({"kind":"actions","state_version":snapshot.version,"actions":[{"name":"create_step","input":{"goal":{"goal_id":id,"reference":null},"text":"Ship prototype"},"instruction":"Add Ship prototype"}],"clarification":null});
    let reasoning=reasoning::parse(&json!({"choices":[{"finish_reason":"stop","message":{"content":reasoning::fixture_plan(plan).to_string()}}]})).unwrap();
    assert!(reasoning
        .validate_for(&event(EventKind::StepChanged, id), &snapshot)
        .is_err());
    reasoning
        .validate_for(&event(EventKind::UserMessage, id), &snapshot)
        .unwrap();
    skills::execute(
        &mut inner.storage,
        &snapshot,
        &reasoning.plan(),
        "Add Ship prototype",
        true,
        false,
    )
    .unwrap();
    assert_eq!(inner.storage.goal_plan(id).unwrap().steps.len(), 3);
    begin_explicit(&mut inner, EventKind::VoiceMessage);
    assert_eq!(
        inner.agent.diagnostics.last_input,
        Some(EventKind::VoiceMessage)
    );
}
#[test]
fn invalid_skill_and_cross_goal_step_offer_are_rejected() {
    let (state, id) = state();
    let inner = state.inner.lock().unwrap();
    let snapshot = skills::snapshot(
        &inner.storage,
        inner.storage.current_conversation().unwrap(),
    )
    .unwrap();
    let mut value = reasoning(&json!({"state_version":snapshot.version}), "offer_skill");
    value["proposed_actions"] = json!([{"name":"run_shell","input":{},"instruction":null}]);
    assert!(reasoning::parse(
        &json!({"choices":[{"finish_reason":"stop","message":{"content":value.to_string()}}]})
    )
    .is_err());
    let action = skills::Action {
        name: "complete_step".into(),
        input: json!({"goal":{"goal_id":id,"reference":null},"step":{"step_id":"other-only-step","ordinal":null,"reference":null}}),
        instruction: None,
    };
    assert!(skills::validate_offer(&inner.storage, &snapshot, &action, Some(id)).is_err());
}
#[test]
fn stale_state_or_privacy_response_is_rejected() {
    for privacy in [false, true] {
        let (state, id) = state();
        let mut inner = state.inner.lock().unwrap();
        inner.agent.enqueue(event(EventKind::StepChanged, id));
        let job = prepare(&mut inner, Utc::now().timestamp())
            .unwrap()
            .unwrap();
        let result = Reasoning::silent(job.snapshot.version.clone());
        if privacy {
            inner.privacy_revision += 1;
        } else {
            let mut plan = inner.storage.goal_plan(id).unwrap();
            plan.steps[0].text = "Corrected".into();
            inner
                .storage
                .core_save_goal("Hackathon", "Work", plan)
                .unwrap();
        }
        assert!(apply(&mut inner, &job, &result).is_err());
        assert_eq!(inner.agent.diagnostics.verified, 0);
    }
}
#[tokio::test]
async fn cancellation_drops_an_in_flight_http_request_and_explicit_input_preempts_it() {
    let (state, id) = state();
    let mut job = {
        let mut inner = state.inner.lock().unwrap();
        inner.agent.enqueue(event(EventKind::StepChanged, id));
        prepare(&mut inner, Utc::now().timestamp())
            .unwrap()
            .unwrap()
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(10)))
        .mount(&server)
        .await;
    let cancel = async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        begin_explicit(&mut state.inner.lock().unwrap(), EventKind::UserMessage);
    };
    let service = service(&state, &server);
    let (result, _) = tokio::join!(reason(&state, &mut job, &service), cancel);
    assert!(result.unwrap_err().contains("cancelled"));
    assert_eq!(state.inner.lock().unwrap().agent.diagnostics.cancelled, 1);
}
#[test]
fn verified_episode_is_remembered_once_and_memory_updates_do_not_loop() {
    let (state, id) = state();
    let mut inner = state.inner.lock().unwrap();
    observe::observe(&mut inner, Utc::now().timestamp()).unwrap();
    inner.agent.pending.clear();
    let mut plan = inner.storage.goal_plan(id).unwrap();
    plan.steps[0].done = true;
    inner
        .storage
        .core_save_goal("Hackathon", "Work", plan)
        .unwrap();
    observe::observe(&mut inner, Utc::now().timestamp()).unwrap();
    inner.agent.pending.clear();
    let mut completion = event(EventKind::CompletionConfirmed, id);
    completion.step_id = Some("step-0".into());
    inner.agent.enqueue(completion.clone());
    let job = prepare(&mut inner, Utc::now().timestamp())
        .unwrap()
        .unwrap();
    let mut result = Reasoning::silent(job.snapshot.version.clone());
    result.decision = Decision::UpdateMemoryOnly;
    result.memory_updates = vec![MemoryUpdate {
        supersedes: None,
        kind: "verified_event".into(),
        content: None,
        event: Some(MemoryEvent::StepCompleted),
        confidence: 1.0,
    }];
    apply(&mut inner, &job, &result).unwrap();
    reasoning::remember_verified(&inner.storage, &completion, &result.memory_updates).unwrap();
    for offset in 1..100 {
        observe::observe(&mut inner, Utc::now().timestamp() + offset).unwrap();
        inner
            .agent
            .pending
            .retain(|e| e.kind != EventKind::MeaningfulContextChanged);
    }
    assert!(inner.agent.pending.is_empty());
    let count: i64 = inner
        .storage
        .connection
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE source_key LIKE 'agent:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let mut own = event(EventKind::StepChanged, id);
    own.origin = Origin::Runtime;
    inner.agent.enqueue(own);
    assert!(inner.agent.pending.is_empty());
}
#[test]
fn unverified_or_mismatched_episode_never_writes_memory() {
    let (state, id) = state();
    let inner = state.inner.lock().unwrap();
    let mut completion = event(EventKind::CompletionConfirmed, id);
    completion.step_id = Some("step-0".into());
    let update = MemoryUpdate {
        supersedes: None,
        kind: "verified_event".into(),
        content: None,
        event: Some(MemoryEvent::StepCompleted),
        confidence: 1.0,
    };
    assert!(reasoning::remember_verified(&inner.storage, &completion, &[update]).is_err());
    assert_eq!(
        inner
            .storage
            .connection
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source_key LIKE 'agent:%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn restart_keeps_cooldown_but_never_replays_requests_or_flights() {
    let path = std::env::temp_dir().join(format!(
        "buddy-agent-{}.db",
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    {
        let s = Storage::open(&path).unwrap();
        s.write_setting(
            "agent_runtime_state",
            &Persisted {
                last_attempt: Utc::now().timestamp(),
            },
        )
        .unwrap();
    }
    {
        let s = Storage::open(&path).unwrap();
        let mut runtime = AgentRuntime::load(&s).unwrap();
        assert!(runtime.flight.is_none());
        assert!(runtime.pending.is_empty());
        runtime.enqueue(event(EventKind::SessionStarted, 1));
        assert!(runtime.ready(Utc::now().timestamp(), true).is_none());
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn stable_context_and_domain_changes_emit_only_significant_events() {
    let (state, id) = state();
    let mut inner = state.inner.lock().unwrap();
    let now = Utc::now().timestamp();
    observe::observe(&mut inner, now).unwrap();
    inner.agent.pending.clear();
    observe::observe(&mut inner, now + 29).unwrap();
    assert!(inner.agent.pending.is_empty());
    observe::observe(&mut inner, now + 30).unwrap();
    assert_eq!(
        inner.agent.pending[0].kind,
        EventKind::MeaningfulContextChanged
    );
    inner.agent.pending.clear();
    inner
        .storage
        .park_work_context(
            id,
            crate::work_context::CheckpointDraft {
                summary: "Outline started".into(),
                next_action: "Continue outline".into(),
                waiting_for: None,
            },
            None,
        )
        .unwrap();
    observe::observe(&mut inner, now + 31).unwrap();
    assert!(inner
        .agent
        .pending
        .iter()
        .any(|e| e.kind == EventKind::WorkContextParked));
    inner.agent.pending.clear();
    inner.storage.resume_work_context(id, true).unwrap();
    observe::observe(&mut inner, now + 32).unwrap();
    assert!(inner
        .agent
        .pending
        .iter()
        .any(|e| e.kind == EventKind::WorkContextResumed));
}
#[test]
fn malformed_reasoning_and_background_preference_guesses_are_rejected() {
    for content in ["{}", "not json", "{\"decision\":\"stay_silent\"}"] {
        assert!(reasoning::parse(
            &json!({"choices":[{"finish_reason":"stop","message":{"content":content}}]})
        )
        .is_err());
    }
    let (state, id) = state();
    let mut inner = state.inner.lock().unwrap();
    inner.agent.enqueue(event(EventKind::GoalChanged, id));
    let job = prepare(&mut inner, Utc::now().timestamp())
        .unwrap()
        .unwrap();
    let mut result = Reasoning::silent(job.snapshot.version.clone());
    result.decision = Decision::UpdateMemoryOnly;
    result.memory_updates = vec![MemoryUpdate {
        supersedes: None,
        kind: "preference".into(),
        content: Some("Likes short replies".into()),
        event: None,
        confidence: 0.9,
    }];
    assert!(apply(&mut inner, &job, &result).is_err());
}

#[test]
fn debounce_waits_for_the_latest_event_and_suspend_clears_work() {
    let mut runtime = AgentRuntime::load(&storage()).unwrap();
    runtime.enqueue(Event::new(EventKind::GoalChanged, Some(1), 1000));
    runtime.enqueue(Event::new(EventKind::StepChanged, Some(1), 1004));
    assert!(runtime.ready(1005, true).is_none());
    assert_eq!(
        runtime.ready(1009, true).unwrap().kind,
        EventKind::StepChanged
    );
    runtime.enqueue(Event::new(EventKind::PossibleDrift, Some(1), 1010));
    runtime.suspend();
    assert!(runtime.pending.is_empty());
    assert!(runtime.ready(1020, true).is_none());
}

#[test]
fn collector_waiting_for_matching_does_not_block_consented_specialist_jobs() {
    let (state, _) = state();
    let mut inner = state.inner.lock().unwrap();
    // The collector withholds foreground while cloud attribution is pending.
    inner.buddy.foreground = None;
    assert!(!background_allowed(&inner));
    assert!(service_allowed(&inner));
    inner.status.tracking = false;
    // A user's queued goal analysis does not require desktop observation.
    assert!(service_allowed(&inner));
    inner.status.dnd = true;
    assert!(!service_allowed(&inner));
}

#[test]
fn offers_require_explicit_chat_and_clarification_displays_the_actual_question() {
    let (state, id) = state();
    let mut inner = state.inner.lock().unwrap();
    inner.agent.enqueue(event(EventKind::StepChanged, id));
    let job = prepare(&mut inner, Utc::now().timestamp())
        .unwrap()
        .unwrap();
    let before = inner.storage.goal_plan(id).unwrap();
    let mut result = Reasoning::silent(job.snapshot.version.clone());
    result.decision = Decision::OfferSkill;
    result.situation = "Would you like to rename this goal?".into();
    result.proposed_actions = vec![skills::Action {
        name: "rename_goal".into(),
        input: json!({"goal":{"goal_id":id,"reference":null},"title":"New name"}),
        instruction: None,
    }];
    assert_eq!(
        apply(&mut inner, &job, &result).unwrap(),
        Some("Would you like help choosing the next action for this goal?".into())
    );
    assert_eq!(
        inner.storage.goal_plan(id).unwrap().revision,
        before.revision
    );
    assert_eq!(inner.storage.goal().unwrap().unwrap().text, "Hackathon");
    inner.agent.saved.last_attempt = 0;
    inner.agent.enqueue(event(EventKind::StepChanged, id));
    let job = prepare(&mut inner, Utc::now().timestamp())
        .unwrap()
        .unwrap();
    let mut result = Reasoning::silent(job.snapshot.version.clone());
    result.decision = Decision::RequestClarification;
    result.clarification = Some("Which step do you mean?".into());
    assert_eq!(
        apply(&mut inner, &job, &result).unwrap(),
        result.clarification
    );
    let offer = VerifiedOffer {
        event: job.event,
        text: "Offer".into(),
        data: job.snapshot.data,
        conversation: job.snapshot.conversation,
        privacy: job.privacy,
    };
    assert!(offer.current(&inner));
    inner.privacy_revision += 1;
    assert!(!offer.current(&inner));
}

#[tokio::test]
async fn mock_provider_is_separate_from_simulated_collection_and_sends_no_http() {
    let (state, id) = state();
    let server = MockServer::start().await;
    mount_silent(&server, 0).await;
    {
        let mut inner = state.inner.lock().unwrap();
        inner.status.demo = true;
        inner.status.mock_ai = true;
        inner.agent.enqueue(event(EventKind::StepChanged, id));
    }
    assert!(run_pending_with_service(&state, &service(&state, &server))
        .await
        .unwrap()
        .is_none());
    let inner = state.inner.lock().unwrap();
    assert_eq!(inner.agent.diagnostics.ai_calls, 0);
    assert_eq!(
        inner.agent.diagnostics.last_decision,
        Some(Decision::StaySilent)
    );
}

#[test]
fn day_start_offer_discards_fabricated_model_accomplishments() {
    let (state, id) = state();
    let mut inner = state.inner.lock().unwrap();
    inner.agent.enqueue(event(EventKind::DayStarted, id));
    let job = prepare(&mut inner, Utc::now().timestamp())
        .unwrap()
        .unwrap();
    assert!(job.source["daily_continuity"]
        .as_str()
        .unwrap()
        .contains("Hackathon"));
    assert!(!job.source.to_string().contains("PRIVATE RAW WINDOW"));
    let mut result = Reasoning::silent(job.snapshot.version.clone());
    result.decision = Decision::OfferHelp;
    result.situation = "You finished the invented deployment yesterday".into();
    let message = apply(&mut inner, &job, &result).unwrap().unwrap();
    assert!(!message.contains("invented deployment"));
    assert!(message.contains("recorded facts"));
    assert!(inner
        .storage
        .goal_plan(id)
        .unwrap()
        .steps
        .iter()
        .all(|s| !s.done));
}
#[test]
fn explicit_no_help_preference_suppresses_reasoning_before_provider_call() {
    let (state, id) = state();
    let mut inner = state.inner.lock().unwrap();
    inner
        .storage
        .create_memory(MemoryInput {
            memory_type: MemoryType::ExplicitPreference,
            scope: Scope::global(),
            content: "Do not offer proactive help".into(),
        })
        .unwrap();
    inner.agent.enqueue(event(EventKind::PossibleDrift, id));
    assert!(prepare(&mut inner, Utc::now().timestamp())
        .unwrap()
        .is_none());
    assert_eq!(inner.agent.diagnostics.ai_calls, 0);
    assert_eq!(
        inner.agent.diagnostics.suppression_reason,
        Some("adaptive_policy")
    );
}

#[tokio::test]
async fn hackathon_idle_stress_has_no_db_writes_inference_or_queue_growth() {
    let (state, _) = state();
    let server = MockServer::start().await;
    mount_silent(&server, 0).await;
    let start = std::time::Instant::now();
    let changes;
    {
        let mut inner = state.inner.lock().unwrap();
        let now = Utc::now().timestamp();
        observe::observe(&mut inner, now).unwrap();
        observe::observe(&mut inner, now + 31).unwrap();
        inner.agent.pending.clear();
        let before = inner.storage.connection.total_changes();
        for _ in 0..1000 {
            observe::observe(&mut inner, now + 32).unwrap();
        }
        changes = inner.storage.connection.total_changes() - before;
        assert_eq!(changes, 0);
        assert!(inner.agent.pending.is_empty());
    }
    assert!(run_pending_with_service(&state, &service(&state, &server))
        .await
        .unwrap()
        .is_none());
    println!("Idle fixture: 1000 stable observations, {} ms wall time, {changes} DB writes, 0 queued events, 0 inference requests", start.elapsed().as_millis());
}

#[tokio::test]
async fn background_repairs_malformed_output_without_explicit_chat_instructions() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (state, id) = state();
    let server = MockServer::start().await;
    let attempts = AtomicUsize::new(0);
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let decisions =
                &body["output_config"]["format"]["schema"]["properties"]["decision"]["enum"];
            assert!(!decisions
                .as_array()
                .unwrap()
                .contains(&json!("execute_explicit_user_action")));
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return response(json!({"invalid":true}));
            }
            let instructions = crate::claude::test::system(&body).as_str().unwrap();
            assert!(instructions.contains("background validation (reasoning_json_invalid)"));
            assert!(!instructions.contains("Copy instruction verbatim from latest_request"));
            let source: Value =
                serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
            response(reasoning(&source, "stay_silent"))
        })
        .expect(2)
        .mount(&server)
        .await;
    state
        .inner
        .lock()
        .unwrap()
        .agent
        .enqueue(event(EventKind::PossibleStuck, id));
    assert!(run_pending_with_service(&state, &service(&state, &server))
        .await
        .unwrap()
        .is_none());
    let inner = state.inner.lock().unwrap();
    assert_eq!(
        inner.agent.diagnostics.last_decision,
        Some(Decision::StaySilent)
    );
    assert_eq!(inner.agent.diagnostics.last_failure, None);
    assert!(inner.agent.flight.is_none());
}

#[tokio::test]
async fn background_failure_is_bounded_and_diagnosed_without_fake_action_or_private_text() {
    for (status, code, requests) in [
        (200, "reasoning_json_invalid", 2),
        (401, "provider_authentication", 1),
        (402, "provider_billing", 1),
    ] {
        let (state, id) = state();
        let server = MockServer::start().await;
        let reply = if status == 200 {
            response(json!({"private":"DO NOT LOG THIS"}))
        } else {
            ResponseTemplate::new(status).set_body_string("DO NOT LOG THIS")
        };
        Mock::given(method("POST"))
            .respond_with(reply)
            .expect(requests)
            .mount(&server)
            .await;
        state
            .inner
            .lock()
            .unwrap()
            .agent
            .enqueue(event(EventKind::PossibleDrift, id));
        assert!(run_pending_with_service(&state, &service(&state, &server))
            .await
            .is_err());
        let mut inner = state.inner.lock().unwrap();
        assert_eq!(inner.agent.diagnostics.last_failure, Some(code));
        assert_eq!(inner.agent.diagnostics.suppression_reason, Some(code));
        assert_eq!(inner.agent.diagnostics.verified, 0);
        assert!(inner.agent.flight.is_none());
        assert!(inner.companion.view.intervention.is_none());
        assert!(!serde_json::to_string(&inner.agent.diagnostics)
            .unwrap()
            .contains("DO NOT LOG THIS"));
        // A failed request still consumes the cooldown; no immediate retry loop.
        inner.agent.enqueue(event(EventKind::PossibleStuck, id));
        assert!(prepare(&mut inner, Utc::now().timestamp())
            .unwrap()
            .is_none());
    }
}
