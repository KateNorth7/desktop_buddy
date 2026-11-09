//! An opt-in live test on synthetic, in-memory goals. Never reads the user's goals or chats.
use super::*;
use crate::storage::Storage;
#[derive(Serialize)]
pub struct CheckResult {
    name: &'static str,
    pub status: &'static str,
    latency_ms: u64,
    diagnostics: Value,
}
fn goal(state: &AppState) -> Option<crate::core::CoreGoal> {
    state
        .inner
        .lock()
        .ok()?
        .storage
        .core_snapshot(None)
        .ok()?
        .goals
        .into_iter()
        .next()
}
async fn send(
    state: &AppState,
    service: &Service<'_>,
    text: &str,
    history: &[Value],
) -> Result<Option<(String, u64, bool)>, String> {
    let (conversation, revision) = {
        let inner = state.inner.lock().unwrap();
        (
            inner.storage.current_conversation().unwrap(),
            inner.privacy_revision,
        )
    };
    let result = crate::agent_runtime::handle_explicit_actions(
        &json!({"latest_request":text,"conversation":history,"event_type":"user_message","natural_reply":true}),
        conversation,
        state,
        revision,
        true,
        service,
    )
    .await;
    result
}
pub async fn run(
    service: &Service<'_>,
    real_state: &AppState,
    revision: u64,
) -> Result<Vec<CheckResult>, String> {
    let isolated = AppState::new(Storage::open(std::path::Path::new(":memory:"))?)?;
    let checks=[
        ("create_goal","Создай тестовую цель «AI CHECK Interview». Пока без шагов."),
        ("create_steps","Добавь туда четыре шага: изучить вакансию; написать ответы; отрепетировать ответы; проверить связь."),
        ("delete_step_preview","Второй удали."),
        ("delete_step_confirm","Подтверждаю."),
        ("complete_step","Первый отметь готовым."),
        ("rename_goal","Переименуй цель «AI CHECK Interview» в «AI CHECK Interview ready»."),
        ("park_context","Сохрани, где я остановился, и отложи цель «AI CHECK Interview ready». Следующий шаг — отрепетировать ответы."),
        ("resume_context","Верни меня к цели «AI CHECK Interview ready»."),
        ("inspect_remaining","Что осталось по цели «AI CHECK Interview ready»? Покажи незавершённые шаги. Ничего не меняй."),
    ];
    let mut results = Vec::new();
    let mut history = Vec::new();
    for (name, text) in checks {
        crate::goal_analysis::ensure_revision(real_state, revision)?;
        {
            let mut inner = real_state
                .inner
                .lock()
                .map_err(|_| "Engine state unavailable")?;
            super::record(&mut inner, "check", name, "started", None, None);
        }
        let before = goal(&isolated);
        let start = Instant::now();
        let received = send(&isolated, service, text, &history).await;
        history.push(json!({"role":"user","text":text}));
        if let Ok(Some((reply, _, _))) = &received {
            history.push(json!({"role":"assistant","text":reply}));
        }
        let g = goal(&isolated);
        let verified = received.as_ref().is_ok_and(|value| value.is_some())
            && match name {
                "create_goal" => g
                    .as_ref()
                    .is_some_and(|g| g.title == "AI CHECK Interview" && g.plan.steps.is_empty()),
                "create_steps" => g.as_ref().is_some_and(|g| g.plan.steps.len() == 4),
                "delete_step_preview" => {
                    let inner = isolated.inner.lock().unwrap();
                    g.as_ref().is_some_and(|g| g.plan.steps.len() == 4)
                        && crate::skills::pending_confirmation(
                            &inner.storage,
                            inner.storage.current_conversation().unwrap(),
                        )
                        .ok()
                        .flatten()
                        .is_some()
                }
                "delete_step_confirm" => g.as_ref().is_some_and(|g| {
                    before.as_ref().is_some_and(|old| {
                        let expected: Vec<_> = old
                            .plan
                            .steps
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| *i != 1)
                            .map(|(_, s)| s.id.as_str())
                            .collect();
                        old.plan.steps.len() == 4
                            && g.plan
                                .steps
                                .iter()
                                .map(|s| s.id.as_str())
                                .collect::<Vec<_>>()
                                == expected
                    })
                }),
                "complete_step" => g
                    .as_ref()
                    .is_some_and(|g| g.plan.steps.len() == 3 && g.plan.steps[0].done),
                "rename_goal" => g
                    .as_ref()
                    .is_some_and(|g| g.title == "AI CHECK Interview ready"),
                "park_context" => g.as_ref().is_some_and(|g| {
                    g.status == "deferred"
                        && g.work_context
                            .as_ref()
                            .is_some_and(|c| c.parked_at.is_some())
                }),
                "resume_context" => g.as_ref().is_some_and(|g| {
                    g.status == "open"
                        && g.work_context
                            .as_ref()
                            .is_some_and(|c| c.resumed_at.is_some())
                }),
                "inspect_remaining" => {
                    let message = received
                        .as_ref()
                        .ok()
                        .and_then(|r| r.as_ref())
                        .map(|r| r.0.to_lowercase())
                        .unwrap_or_default();
                    before
                        .as_ref()
                        .zip(g.as_ref())
                        .is_some_and(|(old, current)| {
                            json!(old) == json!(current)
                                && current.plan.steps.iter().any(|s| !s.done)
                                && current
                                    .plan
                                    .steps
                                    .iter()
                                    .filter(|s| !s.done)
                                    .all(|s| message.contains(&s.text.to_lowercase()))
                        })
                }
                _ => false,
            };
        {
            let mut inner = real_state
                .inner
                .lock()
                .map_err(|_| "Engine state unavailable")?;
            super::record(
                &mut inner,
                "check",
                name,
                if verified { "passed" } else { "failed" },
                Some(elapsed(start)),
                None,
            );
        }
        results.push(CheckResult {
            name,
            status: if verified { "passed" } else { "failed" },
            latency_ms: elapsed(start),
            diagnostics: {
                let inner=isolated.inner.lock().unwrap();
                let refs=inner.storage.action_references(inner.storage.current_conversation().unwrap())?;
                json!({"error":received.as_ref().err().map(|e|super::error_category(e)),"synthetic_receipt":received.as_ref().ok().and_then(|r|r.as_ref()).map(|r|r.0.chars().take(1000).collect::<String>()),"last_inference":inner.engine.last,
                    "last_decision":inner.agent.diagnostics.last_decision,"last_skill":refs.last_action.as_ref().and_then(|r|r["skill"].as_str()),
                    "goal_exists":g.is_some(),"synthetic_title":g.as_ref().map(|g|&g.title),"step_count":g.as_ref().map(|g|g.plan.steps.len())})
            },
        });
        if !verified {
            break;
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{matchers::method, Mock, MockServer, Request};

    #[tokio::test]
    async fn live_check_preserves_history_and_requires_real_delete_confirmation() {
        let state =
            AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(|r: &Request| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            let ctx: Value = serde_json::from_str(b["messages"][0]["content"].as_str().unwrap()).unwrap();
            let name = crate::claude::test::operation(&b).as_str().unwrap_or("buddy_dialogue_response");
            let raw = match name {
                "buddy_skill_plan" => {
                    let (skill,input)=if ctx["latest_request"]=="Create Interview with two steps" {
                        ("create_goal",json!({"title":"Interview","steps":["First","Second"]}))
                    } else {
                        let messages=b["messages"].as_array().unwrap();
                        assert!(messages.iter().any(|m|m["role"]=="user" && m["content"]=="Create Interview with two steps"));
                        assert!(messages.iter().any(|m|m["role"]=="assistant" && m["content"]=="Interview created with two steps."));
                        assert!(!ctx["state"]["references"]["last_referenced_goal_id"].is_null());
                        ("delete_step",json!({"goal":{"goal_id":null,"reference":"last_referenced"},"step":{"step_id":null,"ordinal":2,"reference":null}}))
                    };
                    json!({"state_version":ctx["state"]["state_version"],"response":{"decision":"change","actions":[{"name":skill,"input":input}],"compose_response":true}})
                },
                "buddy_dialogue_authorization"=>json!({"review":{"verdict":"approved","request_mode":"change","message":""}}),
                "buddy_dialogue_confirmation"=>json!({"decision":"confirm"}),
                "buddy_dialogue_response"=>json!({"message":if ctx["latest_request"]=="Create Interview with two steps" {"Interview created with two steps."}else{"Second step deleted."},"resources":[],"memory_update":null,"verified_result_indices":[0]}),
                "buddy_dialogue_review"=>json!({"review":{"verdict":"approved","request_mode":"change","message":""}}),
                other=>panic!("Unexpected operation {other}"),
            };
            crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":raw.to_string()}}]}))
        }).expect(9).mount(&server).await;
        let service = Service {
            client: &state.client,
            endpoint: server.uri(),
            key: "fixture".into(),
            model: "fixture".into(),
        };
        let mut history = Vec::new();
        for text in [
            "Create Interview with two steps",
            "Delete the second one",
            "Confirm",
        ] {
            let reply = send(&state, &service, text, &history)
                .await
                .unwrap()
                .unwrap();
            history.push(json!({"role":"user","text":text}));
            history.push(json!({"role":"assistant","text":reply.0}));
            let inner = state.inner.lock().unwrap();
            let pending = crate::skills::pending_confirmation(
                &inner.storage,
                inner.storage.current_conversation().unwrap(),
            )
            .unwrap();
            if text == "Delete the second one" {
                assert!(pending.is_some());
                assert_eq!(
                    inner.storage.core_snapshot(None).unwrap().goals[0]
                        .plan
                        .steps
                        .len(),
                    2
                );
            } else if text == "Confirm" {
                assert!(pending.is_none());
                let goals = inner.storage.core_snapshot(None).unwrap().goals;
                assert_eq!(goals[0].plan.steps.len(), 1);
                assert_eq!(goals[0].plan.steps[0].text, "First");
            }
        }
    }
}
