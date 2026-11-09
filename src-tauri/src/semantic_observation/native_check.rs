//! Real native collection and provider chain, restricted to an isolated public fixture window.
use super::*;
const TITLE: &str = "Buddy Semantic Fixture";
const MARKER: &str = "SYNTHETIC_ONLY_BUDDY_SEMANTIC";
fn phase(name: &str) {
    #[cfg(all(target_os = "macos", debug_assertions))]
    let _ = std::fs::write("/private/tmp/buddy-semantic-native-phase", name);
    eprintln!("Native semantic check: {name}");
}
fn collect() -> Result<ActivitySnapshot, String> {
    // Unlike the installed Tauri app, this isolated CLI has no AppKit event loop.
    // Drain native workspace notifications before checking the current foreground.
    #[cfg(target_os = "macos")]
    unsafe {
        use std::ffi::c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            static kCFRunLoopDefaultMode: *const c_void;
            fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, once: bool) -> i32;
        }
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
    }
    let mut collector = crate::collector::create(false);
    collector.configure(
        &crate::tracking::TrackingSettings {
            browser_metadata: false,
            ..Default::default()
        },
        &[],
    );
    collector.collect()
}
async fn sample(id: u64) -> Result<(Sample, (u64, String)), String> {
    let a = collect().map_err(|error| format!("Native collection: {error}"))?;
    if a.window_title != TITLE || !crate::collector::session_active() {
        return Err("Activate only the public Buddy Semantic Fixture window".into());
    }
    let window = a.window_id.ok_or("Fixture window missing")?;
    phase(if id == 1 {
        "pending_text"
    } else {
        "confirmed_text"
    });
    let text = companion::sample(window as usize).await?;
    if !text.contains(MARKER) || !crate::collector::matches_foreground(&a) {
        return Err("Public fixture marker or foreground changed".into());
    }
    phase(if id == 1 {
        "pending_pixels"
    } else {
        "confirmed_pixels"
    });
    let image = tauri::async_runtime::spawn_blocking(move || crate::vision::capture(window))
        .await
        .map_err(|_| "Native fixture capture unavailable")??;
    phase(if id == 1 {
        "pending_capture_validation"
    } else {
        "confirmed_capture_validation"
    });
    if !crate::collector::matches_foreground(&a) {
        return Err("Public fixture foreground changed".into());
    }
    Ok((
        Sample {
            id,
            at: Utc::now().timestamp(),
            app: a.process_name,
            window: a.window_id,
            window_title: a.window_title,
            interface_text: bounded(&text, 1800),
            has_image: true,
            foreground_output_active: a.media_playing,
            input_idle_seconds: a.idle_seconds,
            browser: None,
        },
        (id, image),
    ))
}
pub(super) async fn run() -> Result<Value, String> {
    phase("waiting_for_fixture");
    if crate::http::enabled("AI_MOCK") {
        return Err("Native semantic fixture needs real providers".into());
    }
    // A LaunchServices background launch must allow the owner to activate the public fixture.
    // Only metadata is checked while waiting; browser addresses and interface text are disabled.
    eprintln!("Activate Buddy Semantic Fixture; waiting up to 180 seconds before sampling");
    for _ in 0..90 {
        let a = collect()?;
        if a.window_title == TITLE {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    phase("capturing_pending_state");
    let (before, before_image) = match sample(1).await {
        Ok(s) => s,
        Err(error) => {
            return Ok(
                json!({"all_passed":false,"source":"native_public_fixture","cloud_requests":0,"status":error}),
            )
        }
    };
    if !before.interface_text.contains("Payment pending") {
        return Err("Restart the public fixture in its pending state".into());
    }
    phase("awaiting_synthetic_order");
    eprintln!("Native fixture sampled; awaiting Confirm synthetic order (60 seconds)");
    let mut after = None;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let a = collect().map_err(|_| "Native fixture collection unavailable")?;
        if a.window_title != TITLE {
            return Err("Public fixture foreground changed; no cloud disclosure".into());
        }
        let text = companion::sample(a.window_id.ok_or("Fixture window missing")? as usize).await?;
        if !text.contains(MARKER) {
            return Err("Public fixture marker missing".into());
        }
        if text.contains("Order #4231 confirmed just now") {
            phase("capturing_confirmed_state");
            after = Some(sample(2).await?);
            break;
        }
    }
    let (after, after_image) = after.ok_or("Fixture action was not received")?;
    let samples = vec![before, after];
    let images = vec![before_image, after_image];
    let state = AppState::new(crate::storage::Storage::open(std::path::Path::new(
        ":memory:",
    ))?)?;
    let candidates = {
        let mut i = state
            .inner
            .lock()
            .map_err(|_| "Fixture state unavailable")?;
        i.storage.core_create_goal(
            "Complete the synthetic checkout for a chocolate birthday cake with Tuesday pickup",
            "native-cake",
            &chrono::Local::now().date_naive().to_string(),
        )?;
        goals(&i)?
    };
    let preflight = || {
        let a = collect().map_err(|_| "Native collection unavailable")?;
        if a.window_title == TITLE {
            Ok(())
        } else {
            Err("Public fixture foreground changed".into())
        }
    };
    phase("loading_provider");
    let service = Service::configured(&state.client)?;
    let model = {
        let i = state
            .inner
            .lock()
            .map_err(|_| "Fixture state unavailable")?;
        crate::vision::configured_model(&i)
    };
    phase("interpreting_native_evidence");
    let response=infer(&service,&model,Pass::Observation,json!({"open_goals":candidates,"observations":samples,"image_sample_ids":[1,2],"current_time":Utc::now().to_rfc3339(),"relevant_personal_context":null}),&images,&preflight).await?;
    phase("validating_interpretation");
    let answer = parse(&response, &candidates, &samples)?;
    let signal = answer
        .signals
        .iter()
        .find(|s| s.kind == "complete_goal" && s.confidence >= 0.9)
        .ok_or("Native fixture outcome was not detected")?;
    let target = {
        let i = state
            .inner
            .lock()
            .map_err(|_| "Fixture state unavailable")?;
        outcome_target(&i, signal)?
    };
    phase("verifying_native_evidence");
    let response=infer(&service,crate::http::DEFAULT_RESPONSE_MODEL,Pass::Verification,json!({"target":target,"observations":samples,"image_sample_ids":[1,2],"current_time":Utc::now().to_rfc3339()}),&images,&preflight).await?;
    preflight()?;
    let verdict: Verdict = serde_json::from_str(response_text(&response)?)
        .map_err(|_| "Native verifier contract failed")?;
    let layout_wrapped_quote = verdict.evidence.iter().any(|e| {
        !e.image
            && e.quote.as_ref().is_some_and(|q| {
                samples.iter().any(|s| {
                    s.id == e.sample_id
                        && !s.interface_text.contains(q)
                        && literal_quote_in(&s.interface_text, q)
                })
            })
    });
    let mut i = state
        .inner
        .lock()
        .map_err(|_| "Fixture state unavailable")?;
    phase("validating_independent_verdict");
    let Some(proof) = verified_outcome(&i, signal, &verdict, &samples)? else {
        return Ok(
            json!({"all_passed":false,"failure":"independent_verification_rejected","provider_calls":2,
            "current_result":verdict.current_result,"criterion_met":verdict.criterion_met,"established":verdict.established,
            "proposal_grounded":evidence_present(&signal.evidence,&samples,true),
            "verdict_grounded":evidence_present(&verdict.evidence,&samples,true),
            "verdict_evidence":verdict.evidence,
            "missing_evidence":bounded(&verdict.missing_evidence,600)}),
        );
    };
    phase("applying_and_undoing_result");
    let event = i.storage.automatically_complete_outcome(
        &proof,
        "Public native fixture",
        "native-synthetic-order-4231",
    )?;
    let completed = i.storage.core_work_snapshot()?.goals[0].status == "completed";
    i.storage.undo_work_event(event)?;
    let undone = i.storage.core_work_snapshot()?.goals[0].status == "open";
    Ok(
        json!({"all_passed":completed&&undone,"source":"native_public_fixture_real_claude","native_collector":true,"native_interface_text":true,"native_images":2,"provider_calls":2,"completed":completed,"undo_verified":undone,"layout_wrapped_quote":layout_wrapped_quote,"scope":"Only the isolated synthetic window and in-memory goal were used. No installed user state was modified. This verifies the native evidence/provider/mutation chain, not unattended scheduler timing or every third-party app."}),
    )
}
