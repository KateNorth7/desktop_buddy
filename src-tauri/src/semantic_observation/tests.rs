use super::*;

#[test]
fn literal_grounding_accepts_layout_whitespace_but_rejects_paraphrases_and_missing_words() {
    let visible = "Order #4231 confirmed just now.\nChocolate birthday cake.\tTuesday pickup.";
    assert!(literal_quote_in(
        visible,
        "Order #4231 confirmed just now. Chocolate birthday cake."
    ));
    assert!(!literal_quote_in(
        visible,
        "Order #4231 confirmed yesterday."
    ));
    assert!(!literal_quote_in(
        visible,
        "Order #4231 confirmed. Chocolate birthday cake."
    ));
    assert!(!literal_quote_in(
        visible,
        "Chocolate birthday cake. Wednesday pickup."
    ));
    let observations = [sample(2, visible, false)];
    let proof = Evidence {
        sample_id: 2,
        quote: Some("Order #4231 confirmed just now. Chocolate birthday cake.".into()),
        image: false,
    };
    assert!(evidence_present(
        std::slice::from_ref(&proof),
        &observations,
        true
    ));
    assert!(!evidence_present(
        &[Evidence {
            sample_id: 1,
            ..proof
        }],
        &observations,
        true
    ));
}
use crate::storage::Storage;

#[test]
fn observation_rechecks_at_twenty_seconds_and_preserves_pause_and_consent_gates() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.status.tracking = true;
    inner.status.demo = false;
    inner.status.mock_ai = false;
    inner.status.anthropic_configured = true;
    inner.companion.view.preferences.paused = false;
    inner.companion.view.preferences.screen_task_detection = true;
    inner.buddy.foreground = Some(ActivitySnapshot {
        process_name: "Future editor".into(),
        window_title: "A result".into(),
        window_id: Some(42),
        ..Default::default()
    });
    let checked = Instant::now();
    assert!(due_at(&inner, checked));
    inner.companion.semantic.last_check = Some(checked);
    assert!(!due_at(&inner, checked + Duration::from_secs(19)));
    assert!(due_at(&inner, checked + Duration::from_secs(20)));
    inner.companion.view.preferences.paused = true;
    assert!(!due_at(&inner, checked + Duration::from_secs(20)));
    inner.companion.view.preferences.paused = false;
    inner.companion.view.preferences.screen_task_detection = false;
    assert!(!due_at(&inner, checked + Duration::from_secs(20)));
}

fn candidates() -> Value {
    json!([{"id":1,"title":"Order a birthday cake","description":"Chocolate cake for Tuesday","created_at":"2026-10-24T09:00:00Z","plan_revision":0,"done_when":"Order confirmed for Tuesday","steps":[]},
        {"id":2,"title":"Прочитать книгу","description":"Read the entire ebook","created_at":"2026-10-24T09:00:00Z","plan_revision":0,"done_when":"Reach the end of this ebook","steps":[]}])
}
fn sample(id: u64, text: &str, image: bool) -> Sample {
    Sample {
        id,
        at: Utc::now().timestamp(),
        app: "Any future application".into(),
        window: Some(42),
        window_title: "Order details".into(),
        interface_text: text.into(),
        has_image: image,
        foreground_output_active: false,
        input_idle_seconds: 0,
        browser: None,
    }
}
fn answer(signal: Value) -> Value {
    let content = json!({"activity":"Reviewing an outcome","status":"result","matched_goal_id":1,"confidence":0.99,"missing_evidence":"","signals":[signal]});
    json!({"choices":[{"finish_reason":"stop","message":{"content":content.to_string()}}]})
}
fn result() -> Value {
    json!({"kind":"complete_goal","goal_id":1,"step_id":null,"text":null,"confidence":0.99,
        "evidence":[{"sample_id":2,"quote":"Order 4231: chocolate cake, Tuesday pickup confirmed","image":false}]})
}
#[test]
fn semantic_result_uses_exact_current_evidence_without_receipt_keywords_or_current_focus() {
    let samples = vec![
        sample(1, "Checkout: review your cart", false),
        sample(
            2,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            false,
        ),
    ];
    assert_eq!(
        parse(&answer(result()), &candidates(), &samples)
            .unwrap()
            .signals[0]
            .goal_id,
        Some(1)
    );
    let mut russian = result();
    russian["goal_id"] = json!(2);
    russian["evidence"][0]["quote"] = json!("Конец книги — 100 %");
    assert!(parse(
        &answer(russian),
        &candidates(),
        &[sample(2, "Конец книги — 100 %", false)]
    )
    .is_ok());
}
#[test]
fn visual_outcome_requires_actual_pixels_and_rejects_generic_caption_as_quote() {
    let mut s = result();
    s["evidence"][0] = json!({"sample_id":2,"quote":null,"image":true});
    assert!(parse(&answer(s.clone()), &candidates(), &[sample(2, "", false)]).is_err());
    assert!(parse(&answer(s), &candidates(), &[sample(2, "", true)]).is_ok());
    assert!(parse(
        &answer(result()),
        &candidates(),
        &[sample(
            2,
            "A scene that may resemble an order confirmation",
            true
        )]
    )
    .is_err());
}
#[test]
fn old_results_unknown_ids_partial_steps_and_duplicate_signals_are_rejected() {
    let samples = vec![
        sample(
            1,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            false,
        ),
        sample(2, "A different task", false),
    ];
    let mut old = result();
    old["evidence"][0]["sample_id"] = json!(1);
    assert!(parse(&answer(old), &candidates(), &samples).is_err());
    let mut invented = result();
    invented["goal_id"] = json!(999);
    assert!(parse(&answer(invented), &candidates(), &samples).is_err());
    let mut goals = candidates();
    goals[0]["steps"] = json!([{"id":"pay","text":"Pay for order","done":false}]);
    assert!(parse(
        &answer(result()),
        &goals,
        &[sample(
            2,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            false
        )]
    )
    .is_err());
    let mut duplicate = answer(result());
    let mut body: Value = serde_json::from_str(
        duplicate["choices"][0]["message"]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    body["signals"].as_array_mut().unwrap().push(result());
    duplicate["choices"][0]["message"]["content"] = json!(body.to_string());
    assert!(parse(
        &duplicate,
        &candidates(),
        &[sample(
            2,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            false
        )]
    )
    .is_err());
}
#[test]
fn new_goals_and_steps_are_proposals_with_distinct_validated_targets() {
    let mut s = result();
    s["kind"] = json!("new_goal");
    s["goal_id"] = Value::Null;
    s["text"] = json!("Send the requested delivery address");
    assert!(parse(
        &answer(s.clone()),
        &candidates(),
        &[sample(
            2,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            false
        )]
    )
    .is_ok());
    s["goal_id"] = json!(1);
    assert!(parse(
        &answer(s.clone()),
        &candidates(),
        &[sample(
            2,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            false
        )]
    )
    .is_err());
    s["kind"] = json!("new_step");
    assert!(parse(
        &answer(s),
        &candidates(),
        &[sample(
            2,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            false
        )]
    )
    .is_ok());
}
#[test]
fn completing_an_observed_nonfocused_goal_preserves_other_work_and_durable_undo() {
    let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let day = chrono::Local::now().date_naive().to_string();
    storage
        .core_create_goal("Continue the main project", "main-work", &day)
        .unwrap();
    let focused = storage.ensure_today_focus().unwrap().unwrap();
    storage
        .core_create_goal("Order a cake", "cake", &day)
        .unwrap();
    let observed = storage
        .core_work_snapshot()
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.title == "Order a cake")
        .unwrap()
        .id;
    let prompt = Intervention {
        id: "semantic-result".into(),
        kind: Kind::GoalCompletion,
        text: "Order a cake".into(),
        confidence: 0.99,
        goal_id: Some(observed),
        plan_revision: Some(storage.goal_plan(observed).unwrap().revision),
        step_id: None,
        expires_at: Utc::now().timestamp() + 600,
    };
    let event = storage
        .automatically_complete(&prompt, "Future checkout", "immutable-outcome-4231")
        .unwrap();
    assert_eq!(storage.goal().unwrap().unwrap().id, focused);
    assert_eq!(
        storage
            .core_work_snapshot()
            .unwrap()
            .goals
            .into_iter()
            .find(|g| g.id == observed)
            .unwrap()
            .status,
        "completed"
    );
    storage.undo_work_event(event).unwrap();
    assert_eq!(
        storage
            .core_work_snapshot()
            .unwrap()
            .goals
            .into_iter()
            .find(|g| g.id == observed)
            .unwrap()
            .status,
        "open"
    );
    let mut fresh = prompt;
    fresh.plan_revision = Some(storage.goal_plan(observed).unwrap().revision);
    assert!(storage
        .automatically_complete(&fresh, "Future checkout", "immutable-outcome-4231")
        .is_err());
}
#[tokio::test]
async fn multimodal_payload_preserves_pixels_goal_context_and_strict_schema() {
    use wiremock::{
        matchers::{body_partial_json, method, path},
        Mock, MockServer,
    };
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .and(body_partial_json(json!({"model":"synthetic-vision","output_config":{"format":{"type":"json_schema","schema":{"description":"semantic_goal_observation"}}}})))
        .respond_with(crate::claude::test::reply(answer(result()))).expect(1).mount(&server).await;
    let client = reqwest::Client::new();
    let service = Service {
        client: &client,
        endpoint: format!("{}/v1/messages", server.uri()),
        key: "synthetic-unused".into(),
        model: "synthetic-text".into(),
    };
    let value=infer(&service,"synthetic-vision",Pass::Observation,json!({"open_goals":candidates(),"observations":[sample(2,"Order 4231: chocolate cake, Tuesday pickup confirmed",true)]}),
        &[(2,"synthetic-image".into())],&||Ok(())).await.unwrap();
    assert!(parse(
        &value,
        &candidates(),
        &[sample(
            2,
            "Order 4231: chocolate cake, Tuesday pickup confirmed",
            true
        )]
    )
    .is_ok());
    let requests = server.received_requests().await.unwrap();
    let payload: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        payload["messages"][0]["content"][1]["source"]["data"],
        "synthetic-image"
    );
}

#[test]
fn changed_goal_foreground_consent_and_sharing_reject_an_inflight_observation() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.status.tracking = true;
    inner.status.demo = false;
    inner.status.mock_ai = false;
    inner.status.anthropic_configured = true;
    inner.companion.view.preferences.paused = false;
    inner.companion.view.preferences.screen_task_detection = true;
    let activity = ActivitySnapshot {
        process_name: "Future editor".into(),
        window_title: "A result".into(),
        window_id: Some(42),
        ..Default::default()
    };
    inner.buddy.foreground = Some(activity.clone());
    let goals = goals(&inner).unwrap();
    let consent = inner.last_consent_change;
    assert!(context_current(&inner, &activity, consent, &goals));
    inner.last_consent_change += 1;
    assert!(!context_current(&inner, &activity, consent, &goals));
    inner.last_consent_change = consent;
    inner.buddy.foreground.as_mut().unwrap().window_id = Some(99);
    assert!(!context_current(&inner, &activity, consent, &goals));
    inner.buddy.foreground = Some(activity.clone());
    inner
        .storage
        .core_create_goal(
            "Newly edited task",
            "changed-task",
            &chrono::Local::now().date_naive().to_string(),
        )
        .unwrap();
    assert!(!context_current(&inner, &activity, consent, &goals));
    let goals = super::goals(&inner).unwrap();
    let mut ai = inner.storage.ai_preferences().unwrap();
    ai.share_goal_context = false;
    inner
        .storage
        .write_setting("ai_assistance_preferences", &ai)
        .unwrap();
    assert!(!context_current(&inner, &activity, consent, &goals));
}
#[test]
fn captured_results_survive_safe_switches_but_never_privacy_boundaries_or_edits() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.status.tracking = true;
    inner.status.demo = false;
    inner.status.mock_ai = false;
    inner.status.anthropic_configured = true;
    inner.companion.view.preferences.paused = false;
    inner.companion.view.preferences.screen_task_detection = true;
    let captured = ActivitySnapshot {
        process_name: "Public checkout".into(),
        window_title: "Confirmed order".into(),
        window_id: Some(42),
        ..Default::default()
    };
    inner.buddy.foreground = Some(captured.clone());
    let candidates = goals(&inner).unwrap();
    let lease = EvidenceLease {
        consent: inner.last_consent_change,
        epoch: inner.companion.semantic.evidence_epoch,
        captured: Instant::now(),
    };
    assert!(current(&inner, &captured, lease.consent, &candidates));
    inner.buddy.foreground = Some(ActivitySnapshot {
        process_name: "Public notes".into(),
        window_title: "Next task".into(),
        window_id: Some(99),
        ..Default::default()
    });
    // Matching/capture still cannot use the old window; only its sealed result can finish.
    assert!(!current(&inner, &captured, lease.consent, &candidates));
    assert!(lease.valid_in_session(&inner, &candidates, true));
    // Automatic parking/focus UI resets cannot erase a sealed nonfocused result.
    inner.companion.invalidate_for_focus();
    assert!(lease.valid_in_session(&inner, &candidates, true));
    assert!(!lease.valid_in_session(&inner, &candidates, false));
    inner.last_consent_change += 1;
    assert!(!lease.valid_in_session(&inner, &candidates, true));
    inner.last_consent_change = lease.consent;
    let foreground = inner.buddy.foreground.take();
    assert!(!lease.valid_in_session(&inner, &candidates, true));
    inner.buddy.foreground = foreground;
    inner.companion.semantic.invalidate_evidence();
    // Returning from a private/excluded window cannot resurrect the packet.
    assert!(!lease.valid_in_session(&inner, &candidates, true));
    let lease = EvidenceLease {
        epoch: inner.companion.semantic.evidence_epoch,
        ..lease
    };
    assert!(lease.valid_in_session(&inner, &candidates, true));
    let expired = EvidenceLease {
        captured: Instant::now() - Duration::from_secs(121),
        ..lease
    };
    assert!(!expired.valid_in_session(&inner, &candidates, true));
    inner
        .storage
        .core_create_goal(
            "Edited goal",
            "edited-goal",
            &chrono::Local::now().date_naive().to_string(),
        )
        .unwrap();
    assert!(!lease.valid_in_session(&inner, &candidates, true));
}
#[test]
fn terminal_observation_grace_is_bounded_to_the_same_foreground_app() {
    let activity = ActivitySnapshot {
        process_name: "Foreground player".into(),
        ..Default::default()
    };
    let mut runtime = Runtime {
        last_passive: Some((activity.process_name.clone(), Instant::now())),
        ..Default::default()
    };
    assert!(runtime.terminal_grace(&activity));
    assert!(!runtime.terminal_grace(&ActivitySnapshot {
        process_name: "Unrelated notes".into(),
        ..Default::default()
    }));
    runtime.last_passive = Some((
        activity.process_name.clone(),
        Instant::now() - Duration::from_secs(181),
    ));
    assert!(!runtime.terminal_grace(&activity));
    runtime.clear();
    assert!(!runtime.terminal_grace(&activity));
}

#[test]
fn a_detected_new_goal_is_added_only_by_acceptance_and_cannot_duplicate_an_open_goal() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    let proposed = Signal {
        kind: "new_goal".into(),
        goal_id: None,
        step_id: None,
        text: Some("Send the requested delivery address".into()),
        confidence: 0.99,
        evidence: vec![Evidence {
            sample_id: 1,
            quote: Some("Please send the delivery address".into()),
            image: false,
        }],
    };
    let p = prompt(&inner, &proposed).unwrap();
    assert!(inner.storage.core_work_snapshot().unwrap().goals.is_empty());
    companion::confirm_new_goal(&mut inner.storage, &p).unwrap();
    let snapshot = inner.storage.core_work_snapshot().unwrap();
    assert_eq!(snapshot.goals.len(), 1);
    assert_eq!(
        snapshot.goals[0].title,
        "Send the requested delivery address"
    );
    assert_eq!(snapshot.goals[0].status, "open");
    assert!(companion::confirm_new_goal(&mut inner.storage, &p).is_err());
    assert_eq!(inner.storage.core_work_snapshot().unwrap().goals.len(), 1);
    let mut wrong = p;
    wrong.goal_id = Some(snapshot.goals[0].id);
    assert!(companion::confirm_new_goal(&mut inner.storage, &wrong).is_err());
}
#[test]
fn full_completion_criterion_is_not_lost_in_the_bounded_first_pass_catalog() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    let day = chrono::Local::now().date_naive().to_string();
    inner
        .storage
        .core_create_goal("Submit the report", "full-criterion", &day)
        .unwrap();
    let id = inner.storage.ensure_today_focus().unwrap().unwrap();
    let mut plan = inner.storage.goal_plan(id).unwrap();
    plan.done_when = format!(
        "{} Required: signed client approval",
        "Report sections. ".repeat(26)
    );
    inner
        .storage
        .save_goal_plan("Submit the report", plan)
        .unwrap();
    let signal = Signal {
        kind: "complete_goal".into(),
        goal_id: Some(id),
        step_id: None,
        text: None,
        confidence: 0.99,
        evidence: vec![],
    };
    let catalog = goals(&inner).unwrap();
    assert!(!catalog
        .to_string()
        .contains("Required: signed client approval"));
    let target = outcome_target(&inner, &signal).unwrap();
    assert!(target["done_when"]
        .as_str()
        .unwrap()
        .ends_with("Required: signed client approval"));
}

#[test]
fn independent_semantic_proof_requires_current_grounded_agreement_and_preserves_undo() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner
        .storage
        .core_create_goal(
            "Order a birthday cake",
            "proof-cake",
            &chrono::Local::now().date_naive().to_string(),
        )
        .unwrap();
    let id = inner.storage.core_work_snapshot().unwrap().goals[0].id;
    let mut signal: Signal = serde_json::from_value(result()).unwrap();
    signal.goal_id = Some(id);
    signal.confidence = 0.9;
    let samples = vec![sample(
        2,
        "Order 4231: chocolate cake, Tuesday pickup confirmed",
        false,
    )];
    let mut verdict = Verdict {
        current_result: true,
        criterion_met: true,
        established: true,
        evidence: signal.evidence.clone(),
        missing_evidence: String::new(),
    };
    let verified = verified_outcome(&inner, &signal, &verdict, &samples)
        .unwrap()
        .unwrap();
    let event = inner
        .storage
        .automatically_complete_outcome(&verified, "Test checkout", "current-outcome")
        .unwrap();
    assert_eq!(
        inner.storage.core_work_snapshot().unwrap().goals[0].status,
        "completed"
    );
    inner.storage.undo_work_event(event).unwrap();
    assert_eq!(
        inner.storage.core_work_snapshot().unwrap().goals[0].status,
        "open"
    );
    assert!(inner
        .storage
        .automatically_complete_outcome(&verified, "Test checkout", "current-outcome")
        .is_err());
    let fresh = verified_outcome(&inner, &signal, &verdict, &samples)
        .unwrap()
        .unwrap();
    assert!(inner
        .storage
        .automatically_complete_outcome(&fresh, "Test checkout", "changed-image-clock")
        .is_err());
    verdict.established = false;
    assert!(verified_outcome(&inner, &signal, &verdict, &samples)
        .unwrap()
        .is_none());
    verdict.established = true;
    verdict.current_result = false;
    assert!(verified_outcome(&inner, &signal, &verdict, &samples)
        .unwrap()
        .is_none());
    verdict.current_result = true;
    verdict.criterion_met = false;
    assert!(verified_outcome(&inner, &signal, &verdict, &samples)
        .unwrap()
        .is_none());
    verdict.criterion_met = true;
    verdict.evidence[0].quote = Some("Invented quote".into());
    assert!(verified_outcome(&inner, &signal, &verdict, &samples)
        .unwrap()
        .is_none());
    verdict.evidence = vec![Evidence {
        sample_id: 2,
        quote: None,
        image: true,
    }];
    assert!(verified_outcome(&inner, &signal, &verdict, &samples)
        .unwrap()
        .is_none());
    verdict.evidence = vec![Evidence {
        sample_id: 1,
        quote: Some("Old result".into()),
        image: false,
    }];
    let old = vec![sample(1, "Old result", false), samples[0].clone()];
    assert!(verified_outcome(&inner, &signal, &verdict, &old)
        .unwrap()
        .is_none());
}

#[test]
fn nonfocused_step_acceptance_completion_and_undo_keep_focus_and_the_wider_goal_open() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut i = state.inner.lock().unwrap();
    let day = chrono::Local::now().date_naive().to_string();
    i.storage
        .core_create_goal("Continue focused work", "focus", &day)
        .unwrap();
    let focus = i.storage.ensure_today_focus().unwrap().unwrap();
    i.storage
        .core_create_goal("Deliver the budget", "budget", &day)
        .unwrap();
    let id = i
        .storage
        .core_work_snapshot()
        .unwrap()
        .goals
        .iter()
        .find(|g| g.title == "Deliver the budget")
        .unwrap()
        .id;
    let step = companion::append_task(&mut i.storage, Some(id), Some(0), "Send the draft").unwrap();
    assert_eq!(i.storage.goal().unwrap().unwrap().id, focus);
    let signal = Signal {
        kind: "complete_step".into(),
        goal_id: Some(id),
        step_id: Some(step.clone()),
        text: None,
        confidence: 0.9,
        evidence: vec![Evidence {
            sample_id: 1,
            quote: Some("Draft sent just now".into()),
            image: false,
        }],
    };
    let verdict = Verdict {
        current_result: true,
        criterion_met: true,
        established: true,
        evidence: signal.evidence.clone(),
        missing_evidence: String::new(),
    };
    let proof = verified_outcome(
        &i,
        &signal,
        &verdict,
        &[sample(1, "Draft sent just now", false)],
    )
    .unwrap()
    .unwrap();
    let event = i
        .storage
        .automatically_complete_outcome(&proof, "Budget UI", "draft-result")
        .unwrap();
    let goal = i
        .storage
        .core_work_snapshot()
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.id == id)
        .unwrap();
    assert_eq!(goal.status, "open");
    assert!(goal.plan.steps[0].done);
    assert_eq!(i.storage.goal().unwrap().unwrap().id, focus);
    i.storage.undo_work_event(event).unwrap();
    assert!(!i.storage.goal_plan(id).unwrap().steps[0].done);
    assert_eq!(i.storage.goal().unwrap().unwrap().id, focus);
}
#[test]
fn screen_findings_share_the_decision_path_with_the_other_analysers() {
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
    let mut inner = state.inner.lock().unwrap();
    inner.companion.view.preferences.screen_task_detection = true;
    let day = chrono::Local::now().date_naive().to_string();
    inner
        .storage
        .core_create_goal("Order a cake", "cake", &day)
        .unwrap();
    let goal = inner.storage.core_work_snapshot().unwrap().goals[0].id;
    let done = Signal {
        kind: "complete_goal".into(),
        goal_id: Some(goal),
        step_id: None,
        text: None,
        confidence: 0.95,
        evidence: vec![Evidence {
            sample_id: 2,
            quote: Some("Order confirmed".into()),
            image: false,
        }],
    };
    let now = Instant::now();
    let finding = decision(&inner, &done).unwrap();
    assert_eq!(
        finding.plan_revision,
        Some(inner.storage.goal_plan(goal).unwrap().revision)
    );
    assert_eq!(
        crate::decisions::submit_at(&mut inner, finding, now).unwrap(),
        Applied::Queued
    );
    // The episode review reaching the same conclusion later shows no second card.
    let review = Decision {
        source: "episode_review".into(),
        dedupe_key: "review_done:x".into(),
        ..decision(&inner, &done).unwrap()
    };
    assert_eq!(
        crate::decisions::submit_at(&mut inner, review, now).unwrap(),
        Applied::Deduplicated
    );
    // A new goal that is already listed is never offered.
    let listed = Signal {
        kind: "new_goal".into(),
        goal_id: None,
        text: Some("order a cake".into()),
        ..done
    };
    let listed = decision(&inner, &listed).unwrap();
    assert_eq!(
        crate::decisions::submit_at(&mut inner, listed, now).unwrap(),
        Applied::Rejected("already_listed")
    );
}
