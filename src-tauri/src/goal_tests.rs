use super::*;
fn store() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}
#[test]
fn reordered_steps_and_current_step_survive_reopening() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "buddy-plan-test-{}-{unique}.db",
        std::process::id()
    ));
    {
        let mut s = Storage::open(&path).unwrap();
        let g = s.set_goal("Plan").unwrap();
        let mut p = s.goal_plan(g.id).unwrap();
        p.steps = vec![
            Step {
                id: "b".into(),
                text: "Second first".into(),
                done: false,
            },
            Step {
                id: "a".into(),
                text: "Done".into(),
                done: true,
            },
        ];
        p.current_step = Some("b".into());
        p.done_when = "Reviewed".into();
        s.save_goal_plan("Plan", p).unwrap();
    }
    {
        let s = Storage::open(&path).unwrap();
        let g = s.goal().unwrap().unwrap();
        let p = s.goal_plan(g.id).unwrap();
        assert_eq!(p.revision, 1);
        assert_eq!(p.current_step.as_deref(), Some("b"));
        assert_eq!(p.steps[0].id, "b");
        assert!(p.steps[1].done);
        let context: serde_json::Value =
            serde_json::from_str(&s.goal_context(&g).unwrap()).unwrap();
        assert_eq!(context["current_step"], "Second first");
        assert!(context.get("steps").is_none());
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn plan_limits_are_enforced() {
    let mut s = store();
    let g = s.set_goal("Plan").unwrap();
    let mut p = s.goal_plan(g.id).unwrap();
    p.done_when = "x".repeat(501);
    assert!(validate(p.clone()).is_err());
    p.done_when.clear();
    p.steps = (0..21)
        .map(|i| Step {
            id: i.to_string(),
            text: "Step".into(),
            done: false,
        })
        .collect();
    assert!(validate(p.clone()).is_err());
    p.steps.truncate(1);
    p.steps[0].text = " ".into();
    assert!(validate(p).is_err());
}
#[test]
fn plans_validate_and_reject_stale_writes() {
    let mut s = store();
    let g = s.set_goal("Build a page").unwrap();
    let mut p = s.goal_plan(g.id).unwrap();
    p.done_when = "Page is reviewed".into();
    p.steps = vec![Step {
        id: "a".into(),
        text: "Write markup".into(),
        done: false,
    }];
    p.current_step = Some("a".into());
    s.save_goal_plan("Build a landing page", p.clone()).unwrap();
    assert!(s.save_goal_plan("Stale title", p.clone()).is_err());
    assert_eq!(s.goal().unwrap().unwrap().text, "Build a landing page");
    let context = s.goal_context(&s.goal().unwrap().unwrap()).unwrap();
    assert!(context.contains("Write markup"));
    p.revision = 1;
    p.steps[0].done = true;
    assert!(s.save_goal_plan("Build a page", p.clone()).is_err());
    p.current_step = None;
    s.save_goal_plan("Build a page", p.clone()).unwrap();
    p.steps.push(p.steps[0].clone());
    assert!(validate(p).is_err());
}
#[test]
fn switching_defers_and_completion_is_explicit() {
    let mut s = store();
    let a = s.set_goal("First").unwrap();
    let b = s.set_goal("Second").unwrap();
    assert_eq!(s.saved_goals().unwrap()[0].status, "deferred");
    s.transition_goal(a.id, "resume").unwrap();
    assert_eq!(s.goal().unwrap().unwrap().id, a.id);
    assert!(s.transition_goal(b.id, "complete").is_err());
    s.transition_goal(a.id, "complete").unwrap();
    assert!(s.goal().unwrap().is_none());
    assert!(s.transition_goal(a.id, "resume").is_err());
    s.transition_goal(b.id, "resume").unwrap();
}
#[test]
fn retention_keeps_deferred_plans_but_explicit_clear_cascades() {
    let mut s = store();
    let a = s.set_goal("Deferred").unwrap();
    s.save_goal_plan("Deferred", s.goal_plan(a.id).unwrap())
        .unwrap();
    let b = s.set_goal("Active").unwrap();
    s.save_goal_plan("Active", s.goal_plan(b.id).unwrap())
        .unwrap();
    s.connection
        .execute("UPDATE goals SET created_at='2000-01-01T00:00:00Z'", [])
        .unwrap();
    s.purge_history(Some("2020-01-01T00:00:00Z")).unwrap();
    assert_eq!(s.saved_goals().unwrap().len(), 1);
    s.purge_history(None).unwrap();
    assert!(s.saved_goals().unwrap().is_empty());
    let n: i64 = s
        .connection
        .query_row("SELECT COUNT(*) FROM goal_plans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}
#[test]
fn reopening_legacy_database_adds_plans_without_changing_goal() {
    let s = store();
    s.connection.execute_batch("DROP TABLE goal_plans; INSERT INTO goals(text,status,created_at) VALUES('Legacy','active','2026-01-01');").unwrap();
    // Reuse the same initialization path as opening an old database.
    s.initialize().unwrap();
    let g = s.goal().unwrap().unwrap();
    assert_eq!(g.text, "Legacy");
    assert!(s.goal_plan(g.id).unwrap().steps.is_empty());
}
#[test]
fn proposal_parser_rejects_truncation_and_invalid_steps_without_echoing_content() {
    let good = serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":r#"{"title":"Build","done_when":"Reviewed","steps":["Draft"]}"#}}]});
    assert_eq!(parse_proposal(&good).unwrap().steps.len(), 1);
    let mut bad = good.clone();
    bad["choices"][0]["finish_reason"] = serde_json::json!("length");
    assert!(parse_proposal(&bad).is_err());
    bad["choices"][0]["finish_reason"] = serde_json::json!("stop");
    bad["choices"][0]["message"]["content"] = serde_json::json!("private malformed text");
    assert!(!parse_proposal(&bad).unwrap_err().contains("private"));
    for content in [
        r#"{"title":"Build","done_when":"Ready","steps":[]}"#,
        r#"{"title":"Build","done_when":"Ready","steps":[" "]}"#,
        r#"{"title":"Build","done_when":"Ready","steps":["Draft"],"unexpected":true}"#,
    ] {
        bad["choices"][0]["message"]["content"] = serde_json::json!(content);
        assert!(parse_proposal(&bad).is_err());
    }
    let mut refusal = good;
    refusal["choices"][0]["message"]["refusal"] = serde_json::json!("Private refusal");
    assert!(!parse_proposal(&refusal).unwrap_err().contains("Private"));
}
#[tokio::test]
async fn refinement_sends_only_explicit_goal_plan_and_does_not_save() {
    use wiremock::{
        matchers::{header, method},
        Mock, MockServer,
    };
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(header("x-api-key", "test-key"))
        .respond_with(crate::claude::test::reply(serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":r#"{"title":"Refined","done_when":"Reviewed","steps":["Draft"]}"#}}]}))).expect(1).mount(&server).await;
    let mut s = store();
    let g = s.set_goal("Original").unwrap();
    let p = s.goal_plan(g.id).unwrap();
    assert_eq!(
        propose(
            &reqwest::Client::new(),
            &server.uri(),
            "test-key",
            "model",
            &g.text,
            &p
        )
        .await
        .unwrap()
        .title,
        "Refined"
    );
    assert_eq!(s.goal().unwrap().unwrap().text, "Original");
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let context: serde_json::Value =
        serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(context.as_object().unwrap().len(), 3);
    assert!(context.get("recent_activity").is_none());
}

#[test]
fn goal_and_step_text_start_with_a_capital_letter() {
    assert_eq!(
        crate::goals::text("  update the app ", true).unwrap(),
        "Update the app"
    );
    assert_eq!(crate::goals::text("привет", true).unwrap(), "Привет");
    assert_eq!(crate::goals::text("3 things", true).unwrap(), "3 things");
    assert_eq!(
        crate::goals::text("done when shipped", false).unwrap(),
        "done when shipped"
    );
}
