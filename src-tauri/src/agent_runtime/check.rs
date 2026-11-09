//! Explicit, isolated real-provider smoke check. Never opens the user's database.
use super::*;

/// Only called with the isolated demo database, never exposed through Tauri IPC.
pub(crate) async fn hero_stuck(
    state: &AppState,
    service: &Service<'_>,
    goal: i64,
) -> Result<Value, String> {
    {
        let mut inner = state.inner.lock().map_err(|_| "Check state unavailable")?;
        if inner.status.mock_ai {
            return Err("Real-provider check refuses AI_MOCK".into());
        }
        let mut settings = inner.storage.user_settings()?;
        settings.onboarding.completed = true;
        inner.storage.write_setting("user_settings", &settings)?;
        inner.status.anthropic_configured = true;
        inner.status.tracking = true;
        inner.activity_state.state = crate::tracking::ActivityState::Focused;
        inner.companion.view.preferences.screen_task_detection = true;
        inner.companion.view.preferences.quiet_start_minute = 0;
        inner.companion.view.preferences.quiet_end_minute = 0;
        inner.tracking_settings.working_start_minute = 0;
        inner.tracking_settings.working_end_minute = 0;
        inner.buddy.foreground = Some(ActivitySnapshot {
            process_name: "Synthetic editor".into(),
            idle_seconds: 10,
            ..Default::default()
        });
        inner.agent.saved.last_attempt = 0;
        inner.agent.observation.stuck = true;
        inner.agent.enqueue(Event::new(
            EventKind::PossibleStuck,
            Some(goal),
            Utc::now().timestamp() - 10,
        ));
    }
    let offer = process_pending_with_service(state, service, None).await?;
    let inner = state.inner.lock().map_err(|_| "Check state unavailable")?;
    Ok(
        json!({"decision":inner.agent.diagnostics.last_decision,"offer_returned":offer.is_some(),"verified":inner.agent.diagnostics.verified,"failure":inner.agent.diagnostics.last_failure}),
    )
}
use crate::{core::GoalDraft, models::ActivitySnapshot};

pub async fn run() -> Result<Value, String> {
    let mut storage = Storage::open(std::path::Path::new(":memory:"))?;
    storage.core_add_goals(
        vec![GoalDraft {
            title: "Synthetic check: prepare a demo".into(),
            area: "Work".into(),
            steps: vec!["Draft the technical overview".into()],
        }],
        "background-check",
    )?;
    let id = storage.core_snapshot(None)?.goals[0].id;
    storage.core_today(id, true, &Local::now().date_naive().to_string())?;
    storage.transition_goal(id, "resume")?;
    let mut settings = storage.user_settings()?;
    settings.onboarding.completed = true;
    storage.write_setting("user_settings", &settings)?;
    let state = AppState::new(storage)?;
    {
        let mut inner = state.inner.lock().map_err(|_| "Check state unavailable")?;
        if inner.status.mock_ai {
            return Err("Real-provider check refuses AI_MOCK".into());
        }
        inner.status.anthropic_configured = true;
        inner.status.tracking = true;
        inner.activity_state.state = crate::tracking::ActivityState::Focused;
        inner.companion.view.preferences.screen_task_detection = true;
        inner.companion.view.preferences.quiet_start_minute = 0;
        inner.companion.view.preferences.quiet_end_minute = 0;
        inner.tracking_settings.working_start_minute = 0;
        inner.tracking_settings.working_end_minute = 0;
        inner.buddy.foreground = Some(ActivitySnapshot {
            process_name: "Synthetic editor".into(),
            window_title: String::new(),
            idle_seconds: 10,
            ..Default::default()
        });
    }
    let service = Service::configured(&state.client)?;
    let mut checks = vec![];
    for kind in [EventKind::PossibleDrift, EventKind::PossibleStuck] {
        let verified_before = {
            let mut inner = state.inner.lock().map_err(|_| "Check state unavailable")?;
            inner.agent.saved.last_attempt = 0;
            inner.agent.observation.drift = kind == EventKind::PossibleDrift;
            inner.agent.observation.stuck = kind == EventKind::PossibleStuck;
            inner.activity_state.state = if kind == EventKind::PossibleDrift {
                crate::tracking::ActivityState::Drifting
            } else {
                crate::tracking::ActivityState::Focused
            };
            inner
                .agent
                .enqueue(Event::new(kind, Some(id), Utc::now().timestamp() - 10));
            inner.agent.diagnostics.verified
        };
        let started = std::time::Instant::now();
        let result = process_pending_with_service(&state, &service, None).await;
        let inner = state.inner.lock().map_err(|_| "Check state unavailable")?;
        checks.push(json!({"event":kind,"passed":result.is_ok() && inner.agent.diagnostics.verified == verified_before + 1,
            "decision":inner.agent.diagnostics.last_decision,"offer_returned":result.is_ok_and(|o|o.is_some()),
            "latency_ms":started.elapsed().as_millis(),"failure":inner.agent.diagnostics.last_failure}));
    }
    Ok(
        json!({"synthetic_data_only":true,"all_passed":checks.iter().all(|c| c["passed"]==true),"checks":checks}),
    )
}
