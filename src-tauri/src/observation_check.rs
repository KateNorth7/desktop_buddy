//! Opt-in live-provider checks use only synthetic fixtures and an isolated in-memory database.
use crate::{commands::AppState, goal_analysis::Service, storage::Storage};
use base64::Engine;
use serde_json::{json, Value};
use std::time::Instant;
pub(crate) async fn run() -> Result<Value, String> {
    if std::env::args().any(|a| a == "--native-fixture") {
        return native_fixture().await;
    }
    if crate::http::enabled("AI_MOCK") {
        return Err("Live observation check requires real providers".into());
    }
    let state = AppState::new(Storage::open(std::path::Path::new(":memory:"))?)?;
    let service = Service::configured(&state.client)?;
    let mut rows = Vec::new();
    let receipts_only = std::env::args().any(|a| a == "--receipt-only");
    if !receipts_only {
        let unplanned = crate::unplanned_work::check::run().await;
        rows.push(json!({"name":"unlisted_work_classification","passed":unplanned.as_ref().is_ok_and(|v|v["all_passed"]==true)}));
    }
    for (name,criterion,text,quote,expected) in [
        ("fresh_receipt","Send the interview follow-up","Gmail. Interview follow-up to the recruiter. Fresh notification: Message sent successfully. Undo.","Message sent successfully",true),
        ("draft_is_not_complete","Send the interview follow-up","Gmail compose draft. Body: interview follow-up. Draft saved successfully. Send button enabled.","Draft saved successfully",false),
        ("old_sent_history_is_not_complete","Send the interview follow-up","Sent folder history from last week. Message sent successfully. Date: 2026-10-01. No new action performed.","Message sent successfully",false),
        ("quoted_receipt_is_not_complete","Send the interview follow-up","Reading a received email: the trainer wrote 'Message sent successfully' as an example. This is a quote, not a current result.","Message sent successfully",false)
    ] {
        let at=Instant::now();let result=crate::work_journal::receipt_verdict(&service,criterion,text,quote,&||Ok(())).await;
        rows.push(json!({"name":name,"passed":result.as_ref().is_ok_and(|v|v.as_ref().is_some_and(|v|v.accepted())==expected),"latency_ms":at.elapsed().as_millis(),"verdict":result.as_ref().ok(),"error":result.err().map(|e|crate::engine_observability::error_category(&e))}));
    }
    if !receipts_only {
        let goal = {
            let mut i = state.inner.lock().unwrap();
            i.storage.core_create_goal(
                "Send the interview follow-up",
                "live-observation-fixture",
                &chrono::Local::now().date_naive().to_string(),
            )?;
            let id = i
                .storage
                .ensure_today_focus()?
                .ok_or("Fixture goal unavailable")?;
            let mut plan = i.storage.goal_plan(id)?;
            plan.done_when = "Follow-up delivered to the recruiter".into();
            plan.steps = vec![crate::goals::Step {
                id: "follow-up".into(),
                text: "Send the interview follow-up".into(),
                done: false,
            }];
            i.storage
                .save_goal_plan("Send the interview follow-up", plan)?;
            id
        };
        let plan = state.inner.lock().unwrap().storage.goal_plan(goal)?;
        let text="Gmail. Subject: Interview follow-up. To: recruiter@example.test. Message sent successfully. Undo.";
        let at = Instant::now();
        let signal = crate::companion::classify_visible_text(
            &state,
            &service,
            "Send the interview follow-up",
            &plan,
            "Browser",
            text,
        )
        .await;
        let mut passed = false;
        let mut signal_kind = None;
        if let Ok(signal) = signal {
            signal_kind = Some(signal.kind.clone());
            if signal.kind == "completion"
                && signal.confidence >= crate::work_journal::MIN_VERIFIED_CONFIDENCE
            {
                let quote = signal.evidence.as_deref().unwrap_or_default();
                if crate::work_journal::verify_receipt(
                    &service,
                    "Send the interview follow-up",
                    text,
                    quote,
                    &|| Ok(()),
                )
                .await?
                {
                    let mut i = state.inner.lock().unwrap();
                    let prompt = crate::companion::engine::Intervention {
                        id: "live-synthetic".into(),
                        kind: crate::companion::engine::Kind::Completion,
                        text: "Send the interview follow-up".into(),
                        confidence: signal.confidence,
                        goal_id: Some(goal),
                        plan_revision: Some(plan.revision),
                        step_id: signal.step_id,
                        expires_at: i64::MAX,
                    };
                    let event = i
                        .storage
                        .automatically_complete(&prompt, "Browser", quote)?;
                    passed = i.storage.goal_plan(goal)?.steps[0].done;
                    i.storage.undo_work_event(event)?;
                    passed = passed && !i.storage.goal_plan(goal)?.steps[0].done;
                }
            }
        }
        rows.push(json!({"name":"real_model_signal_rust_completion_and_undo","passed":passed,"signal_kind":signal_kind,"latency_ms":at.elapsed().as_millis()}));
    }
    if !receipts_only {
        let at = Instant::now();
        let image =
            base64::engine::general_purpose::STANDARD.encode(include_bytes!("vision/fixture.jpg"));
        let vision = crate::vision::request_image(&service, &image, &|| Ok(())).await;
        let vision_text = vision
            .as_ref()
            .ok()
            .and_then(|v| crate::goal_analysis::response_text(v).ok())
            .unwrap_or("")
            .to_lowercase();
        rows.push(json!({"name":"real_image_understanding","passed":(vision_text.contains("draft")||vision_text.contains("compos"))&&vision_text.contains("follow"),"latency_ms":at.elapsed().as_millis(),"provider_reported_model":vision.as_ref().ok().and_then(|v|v["model"].as_str()),"error":vision.err().map(|e|crate::engine_observability::error_category(&e))}));
        let chat = state.inner.lock().unwrap().storage.create_conversation()?;
        let text="Remember my work context: I work on product design, and I build a side project called Beacon.";
        let at = Instant::now();
        let outcome=crate::agent_runtime::handle_explicit_actions(&json!({"latest_request":text,"conversation":[],"event_type":"user_message","natural_reply":true}),chat,&state,0,true,&service).await;
        let facts =
            state
                .inner
                .lock()
                .unwrap()
                .storage
                .list_memories(crate::memory::MemoryQuery {
                    memory_type: Some(crate::memory::MemoryType::ExplicitFact),
                    ..Default::default()
                })?;
        rows.push(json!({"name":"explicit_knowledge_through_shared_dialogue","passed":outcome.is_ok()&&!facts.is_empty(),"latency_ms":at.elapsed().as_millis(),"saved_fact_count":facts.len()}));
    }
    Ok(
        json!({"all_passed":rows.iter().all(|r|r["passed"]==true),"provider":"real_claude","fixture":"synthetic_only_in_memory","cases":rows,"scope":"No installed goals, chats, screen contents or screenshots were read. Native capture, OS permissions and physical microphone remain separate manual checks."}),
    )
}

/// Local-only native probe. Refuse every foreground window except the public test fixture.
async fn native_fixture() -> Result<Value, String> {
    let sample = match crate::collector::create(false).collect() {
        Ok(sample) => sample,
        Err(_) => {
            #[cfg(target_os = "macos")]
            let accessibility = crate::macos::get_accessibility_permission();
            #[cfg(not(target_os = "macos"))]
            let accessibility = true;
            return Ok(
                json!({"all_passed":false,"fixture":"public_native_window_only","cloud_requests":0,"status":"foreground_collection_unavailable_check_accessibility_permission","accessibility_permission":accessibility,"interactive_session":crate::collector::session_active()}),
            );
        }
    };
    if sample.window_title != "Buddy Observation Fixture" {
        return Ok(
            json!({"all_passed":false,"fixture":"public_native_window_only","cloud_requests":0,"status":"activate_public_fixture_first"}),
        );
    }
    let window = sample.window_id.ok_or("Fixture window is unavailable")?;
    let text = crate::companion::sample(window as usize).await;
    #[cfg(target_os = "macos")]
    let permission = crate::macos::screen_permission(
        std::env::args().any(|a| a == "--request-screen-permission"),
    );
    #[cfg(not(target_os = "macos"))]
    let permission = true;
    let image = if permission {
        tauri::async_runtime::spawn_blocking(move || crate::vision::capture(window))
            .await
            .map_err(|_| "Native capture unavailable")?
    } else {
        Err(crate::vision::CaptureFailure::new(
            crate::vision::CaptureFailureKind::PermissionUnavailable,
            "system_screen_permission_missing",
        ))
    };
    Ok(
        json!({"fixture":"public_native_window_only","cloud_requests":0,"screen_permission":permission,
        "accessible_text_available":text.as_ref().is_ok_and(|s|s.contains("BEACON")),"image_captured":image.is_ok(),
        "capture_status":image.as_ref().err().map(|f|f.status()),"capture_detail":image.as_ref().err().map(|f|f.detail),
        "image_base64_bytes":image.as_ref().ok().map(|s|s.len()),"all_passed":text.as_ref().is_ok_and(|s|s.contains("BEACON"))&&image.is_ok(),
        "scope":"No real application content, API keys or provider requests are used by this native probe. Image data is discarded, not printed or saved."}),
    )
}
