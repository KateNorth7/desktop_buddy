use super::*;
use crate::{
    goals::Step,
    memory::{MemoryInput, MemoryType, Scope},
};
fn store() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}
fn goal(s: &mut Storage) -> i64 {
    s.core_create_goal(
        "Send the interview follow-up",
        "fixture",
        &Local::now().date_naive().to_string(),
    )
    .unwrap();
    let id = s.ensure_today_focus().unwrap().unwrap();
    let mut plan = s.goal_plan(id).unwrap();
    plan.done_when = "Interview follow-up sent to the recruiter".into();
    plan.steps = vec![Step {
        id: "followup".into(),
        text: "Send the interview follow-up".into(),
        done: false,
    }];
    s.save_goal_plan("Send the interview follow-up", plan)
        .unwrap();
    id
}
fn prompt(s: &Storage, id: i64) -> Intervention {
    Intervention {
        id: "fixture".into(),
        kind: Kind::Completion,
        text: "Send the interview follow-up".into(),
        confidence: 0.99,
        goal_id: Some(id),
        plan_revision: Some(s.goal_plan(id).unwrap().revision),
        step_id: Some("followup".into()),
        expires_at: i64::MAX,
    }
}
use chrono::Local;
#[test]
fn automatic_step_is_grounded_reversible_and_does_not_complete_the_wider_goal() {
    let mut s = store();
    let id = goal(&mut s);
    let p = prompt(&s, id);
    let event = s
        .automatically_complete(&p, "Browser", "Message sent: interview follow-up")
        .unwrap();
    s.core_settle_goals().unwrap();
    assert!(s.goal_plan(id).unwrap().steps[0].done);
    assert_ne!(s.core_snapshot(None).unwrap().goals[0].status, "completed");
    assert_eq!(
        s.work_journal(&Local::now().date_naive().to_string())
            .unwrap()[0]
            .source,
        "automatic_verified"
    );
    s.undo_work_event(event).unwrap();
    assert!(!s.goal_plan(id).unwrap().steps[0].done);
    assert!(s.undo_work_event(event).is_err());
    assert!(s
        .automatically_complete(
            &prompt(&s, id),
            "Browser",
            "Message sent: interview follow-up"
        )
        .is_err());
}
#[test]
fn stale_completion_and_undo_never_overwrite_a_new_plan() {
    let mut s = store();
    let id = goal(&mut s);
    let stale = prompt(&s, id);
    let mut changed = s.goal_plan(id).unwrap();
    changed.steps[0].text = "Send the revised follow-up".into();
    s.save_goal_plan("Send the interview follow-up", changed)
        .unwrap();
    assert!(s
        .automatically_complete(&stale, "Browser", "Message sent: interview follow-up")
        .is_err());
    let e = s
        .automatically_complete(
            &prompt(&s, id),
            "Browser",
            "Message sent: revised interview follow-up",
        )
        .unwrap();
    let mut plan = s.goal_plan(id).unwrap();
    plan.done_when = "New criterion".into();
    s.save_goal_plan("Send the interview follow-up", plan)
        .unwrap();
    assert!(s.undo_work_event(e).is_err());
    assert_eq!(s.goal_plan(id).unwrap().done_when, "New criterion");
}
#[test]
fn memory_off_disables_profile_writes_and_personal_clear_preserves_goals_and_journal() {
    let mut s = store();
    let id = goal(&mut s);
    let chat = s.create_conversation().unwrap();
    let statement = "I work on product design and a side project.";
    s.store_user_knowledge(statement, statement, chat, MemoryType::ExplicitFact, 0.99)
        .unwrap();
    s.store_user_knowledge(statement, statement, chat, MemoryType::ExplicitFact, 0.99)
        .unwrap();
    assert_eq!(
        s.list_memories(crate::memory::MemoryQuery {
            memory_type: Some(MemoryType::ExplicitFact),
            ..Default::default()
        })
        .unwrap()
        .len(),
        1
    );
    assert!(s
        .store_user_knowledge(
            "Invented employer",
            statement,
            chat,
            MemoryType::ExplicitFact,
            0.99
        )
        .is_err());
    s.record_work_event(
        "observed-1",
        None,
        None,
        "Prepare a follow-up",
        "Browser",
        crate::work_journal::Outcome::Observed,
    )
    .unwrap();
    let mut controls = s.memory_controls().unwrap();
    controls.enabled = false;
    s.set_memory_controls(&controls).unwrap();
    assert!(s
        .store_user_knowledge(statement, statement, chat, MemoryType::ExplicitFact, 0.99)
        .is_err());
    s.clear_personal_memory().unwrap();
    assert_eq!(s.goal_plan(id).unwrap().goal_id, id);
    assert_eq!(
        s.work_journal(&Local::now().date_naive().to_string())
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn journal_deduplicates_and_never_stores_screen_text() {
    let s = store();
    let title = "Prepare interview follow-ups";
    let a = s
        .record_work_event(
            "receipt-fingerprint",
            None,
            None,
            title,
            "Browser",
            crate::work_journal::Outcome::Observed,
        )
        .unwrap();
    let b = s
        .record_work_event(
            "receipt-fingerprint",
            None,
            None,
            title,
            "Browser",
            crate::work_journal::Outcome::Observed,
        )
        .unwrap();
    assert_eq!(a, b);
    let events = s
        .work_journal(&Local::now().date_naive().to_string())
        .unwrap();
    assert_eq!(events.len(), 1);
    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains("visible_text"));
    assert!(!encoded.contains("screenshot"));
    assert!(s
        .record_work_event(
            "bad",
            None,
            None,
            "api_key=secret-canary",
            "Browser",
            crate::work_journal::Outcome::Observed
        )
        .is_err());
}
#[tokio::test]
async fn receipt_verifier_rejects_visual_descriptions_and_unquoted_evidence_before_http() {
    let client = reqwest::Client::new();
    let service = crate::goal_analysis::Service {
        client: &client,
        endpoint: "http://127.0.0.1:1".into(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    assert!(!verify_receipt(
        &service,
        "Send follow-up",
        "Compose email\n[Visual context — unverified]\nMessage sent successfully",
        "Message sent successfully",
        &|| Ok(())
    )
    .await
    .unwrap());
    assert!(!verify_receipt(
        &service,
        "Send follow-up",
        "Compose email",
        "Message sent successfully",
        &|| Ok(())
    )
    .await
    .unwrap());
}
#[tokio::test]
async fn receipt_verifier_requires_current_exact_criterion_and_high_confidence() {
    use wiremock::{matchers::method, Mock, MockServer};
    let server = MockServer::start().await;
    let client = reqwest::Client::new();
    let service = crate::goal_analysis::Service {
        client: &client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture".into(),
    };
    for (current, criterion, confidence, expected) in [
        (true, true, 0.99, true),
        (true, true, 0.95, true),
        (true, true, 0.94, false),
        (false, true, 0.99, false),
        (true, false, 0.99, false),
        (true, true, 0.9, false),
    ] {
        server.reset().await;
        Mock::given(method("POST")).respond_with(crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"current_receipt":current,"criterion_met":criterion,"confidence":confidence}).to_string()}}]}))).expect(1).mount(&server).await;
        assert_eq!(
            verify_receipt(
                &service,
                "Send follow-up",
                "Message sent successfully",
                "Message sent successfully",
                &|| Ok(())
            )
            .await
            .unwrap(),
            expected
        );
    }
}
#[test]
fn scoped_daily_summary_does_not_leak_other_lanes_or_outside_work() {
    let mut s = store();
    let id = goal(&mut s);
    s.create_memory(MemoryInput {
        memory_type: MemoryType::ExplicitFact,
        scope: Scope::global(),
        content: "I work on product design".into(),
    })
    .unwrap();
    s.record_work_event(
        "outside",
        None,
        None,
        "Unrelated private project",
        "Browser",
        crate::work_journal::Outcome::Observed,
    )
    .unwrap();
    let report = s
        .continuity_report(
            crate::continuity::View::Remaining,
            Some(id),
            Local::now().date_naive(),
        )
        .unwrap();
    assert!(!report.render().contains("Unrelated private project"));
    assert!(report.work_events.is_empty());
}

#[test]
fn user_corrections_supersede_old_profile_facts_without_touching_goals() {
    let mut s = store();
    let id = goal(&mut s);
    let chat = s.create_conversation().unwrap();
    let old = s
        .create_memory(MemoryInput {
            memory_type: MemoryType::ExplicitFact,
            scope: Scope::global(),
            content: "I work in marketing".into(),
        })
        .unwrap();
    s.correct_user_knowledge(
        "I now work in product design",
        "I now work in product design",
        chat,
        MemoryType::ExplicitFact,
        0.99,
        Some(old.id),
    )
    .unwrap();
    assert!(s.memory(old.id, false).unwrap().archived);
    assert_eq!(s.goal_plan(id).unwrap().goal_id, id);
    let facts = s
        .list_memories(crate::memory::MemoryQuery {
            memory_type: Some(MemoryType::ExplicitFact),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].content, "I now work in product design");
}
#[test]
fn task_patterns_need_multiple_confirmed_days_and_stay_out_of_retrieval_until_review() {
    let mut s = store();
    for index in 0..3 {
        s.core_create_goal(
            &format!("Fixture {index}"),
            &format!("pattern-{index}"),
            &Local::now().date_naive().to_string(),
        )
        .unwrap();
        let id = s
            .core_snapshot(None)
            .unwrap()
            .goals
            .iter()
            .find(|g| g.title == format!("Fixture {index}"))
            .unwrap()
            .id;
        s.observe_typical_work(id, "Handle interview follow-ups", true)
            .unwrap();
        if index < 2 {
            s.connection
                .execute(
                    "UPDATE memories SET created_at=datetime('now',?1) WHERE source_key=?2",
                    params![
                        format!("-{} days", index + 1),
                        format!("unplanned-confirmed-{id}")
                    ],
                )
                .unwrap();
        }
    }
    let patterns = s
        .list_memories(crate::memory::MemoryQuery {
            memory_type: Some(MemoryType::BehavioralPattern),
            include_archived: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(patterns.len(), 1);
    assert!(patterns[0].archived);
    assert_eq!(patterns[0].evidence_count, 3);
    assert!(s
        .list_memories(crate::memory::MemoryQuery {
            memory_type: Some(MemoryType::BehavioralPattern),
            ..Default::default()
        })
        .unwrap()
        .is_empty());
    s.archive_memory(patterns[0].id, false).unwrap();
    assert!(!s.memory(patterns[0].id, false).unwrap().archived);
}

#[test]
fn restart_preserves_verified_results_undo_and_next_day_history() {
    let path = std::env::temp_dir().join(format!(
        "buddy-journal-{}-{}.sqlite",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let (goal_id, event) = {
        let mut s = Storage::open(&path).unwrap();
        let id = goal(&mut s);
        let event = s
            .automatically_complete(
                &prompt(&s, id),
                "Browser",
                "Message sent: follow-up delivered",
            )
            .unwrap();
        (id, event)
    };
    {
        let mut s = Storage::open(&path).unwrap();
        assert!(s.goal_plan(goal_id).unwrap().steps[0].done);
        let day = Local::now().date_naive();
        let report = s
            .continuity_report(
                crate::continuity::View::Yesterday,
                None,
                day + chrono::Duration::days(1),
            )
            .unwrap();
        assert!(report
            .lanes
            .iter()
            .any(|l| l.completed.iter().any(|c| c.source == "automatic_verified")));
        s.undo_work_event(event).unwrap();
        assert!(!s.goal_plan(goal_id).unwrap().steps[0].done);
    }
    let _ = std::fs::remove_file(path);
}
#[tokio::test]
async fn foreground_or_consent_change_cancels_an_inflight_observation_without_retry() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(3))
                .set_body_json(json!({})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let valid = AtomicBool::new(true);
    let check = || {
        if valid.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("Observation consent changed".into())
        }
    };
    let client = reqwest::Client::new();
    let payload = json!({"fixture":true});
    let uri = server.uri();
    let request = crate::http::post_guarded_json(&client, &uri, "fixture", &payload, &check);
    let revoke = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        valid.store(false, Ordering::SeqCst);
    };
    let (result, _) = tokio::join!(request, revoke);
    assert!(result.unwrap_err().contains("changed"));
}

#[test]
fn background_snapshot_preserves_lanes_without_rebuilding_progress() {
    let mut s = store();
    goal(&mut s);
    for index in 0..5 {
        s.core_create_goal(
            &format!("Parallel lane {index}"),
            &format!("lane-{index}"),
            &Local::now().date_naive().to_string(),
        )
        .unwrap();
    }
    let full = s.core_snapshot(None).unwrap();
    let light = s.core_work_snapshot().unwrap();
    assert_eq!(
        serde_json::to_value(&full.goals).unwrap(),
        serde_json::to_value(&light.goals).unwrap()
    );
    assert_eq!(full.today, light.today);
    assert_eq!(full.focused_goal_id, light.focused_goal_id);
    assert_eq!(full.date, light.date);
    assert!(!full.week.is_empty());
    assert!(light.week.is_empty());
    assert!(light.hourly.is_empty());
    let at = std::time::Instant::now();
    for _ in 0..30 {
        s.core_snapshot(None).unwrap();
    }
    let full_time = at.elapsed();
    let at = std::time::Instant::now();
    for _ in 0..30 {
        s.core_work_snapshot().unwrap();
    }
    let light_time = at.elapsed();
    println!(
        "BACKGROUND_SNAPSHOT_BENCHMARK {}",
        json!({"fixture":"six_lanes_in_memory","iterations":30,"full_snapshot_us":full_time.as_micros(),"work_snapshot_us":light_time.as_micros(),"ai_calls":0})
    );
}
