use super::*;
use crate::{models::*, storage::Storage};
use wiremock::{
    matchers::{header, method},
    Mock, MockServer, ResponseTemplate,
};

#[test]
fn endpoints_and_retention_are_validated() {
    assert_eq!(
        test_url(
            "https://api.anthropic.com/v1/messages",
            "anthropic",
            Some("claude-opus-5-5")
        )
        .unwrap()
        .as_str(),
        "https://api.anthropic.com/v1/models/claude-opus-5-5"
    );
    assert_eq!(
        test_url("https://api.tavily.com/search", "tavily", None)
            .unwrap()
            .as_str(),
        "https://api.tavily.com/usage"
    );
    assert!(test_url("https://example.com/unknown", "anthropic", Some("claude-opus-5-5")).is_err());
    assert!(test_url("https://api.anthropic.com/v1/messages", "anthropic", Some("../secret")).is_err());
    for days in [0, 7, 30, 90] {
        assert!(validate_retention(days).is_ok());
    }
    assert!(validate_retention(1).is_err());
}
#[test]
fn clearing_history_preserves_active_goal_and_settings() {
    let mut store = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let old = store.set_goal("Old goal").unwrap();
    store
        .record_recommendation(old.id, "A", "https://example.com/a", "Useful")
        .unwrap();
    let goal = store.set_goal("Current goal").unwrap();
    let snapshot = ActivitySnapshot {
        timestamp: chrono::Utc::now().to_rfc3339(),
        process_name: "editor.exe".into(),
        window_title: "Draft".into(),
        idle_seconds: 10,
        active_seconds: 60,
        ..Default::default()
    };
    store.activity(goal.id, &snapshot).unwrap();
    let mut decision = Decision {
        id: None,
        state: FocusState::Focused,
        confidence: 0.9,
        reason: "Writing".into(),
        action: Action::Wait,
    };
    store.decision(goal.id, &mut decision).unwrap();
    store.feedback(decision.id.unwrap(), true).unwrap();
    store.write_setting("retention_days", &30).unwrap();
    store
        .write_setting("buddy_position", &Option::<BuddyPosition>::None)
        .unwrap();
    assert_eq!(
        store
            .read_setting::<BuddyPosition>("buddy_position")
            .unwrap(),
        None
    );
    store.purge_history(None).unwrap();
    assert!(store.recent(goal.id).unwrap().is_empty());
    assert!(store.recommendation_history().unwrap().is_empty());
    assert!(store.latest_decision(goal.id).unwrap().is_none());
    assert_eq!(store.goal().unwrap().unwrap().text, "Current goal");
    assert_eq!(
        store.read_setting::<u32>("retention_days").unwrap(),
        Some(30)
    );
    let goals: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM goals", [], |r| r.get(0))
        .unwrap();
    assert_eq!(goals, 1);
}
#[test]
fn retention_removes_only_old_records() {
    let mut store = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let goal = store.set_goal("Learn").unwrap();
    let old = store
        .record_recommendation(goal.id, "Old", "https://example.com/old", "Old result")
        .unwrap();
    store.rate_recommendation(old, Some(false)).unwrap();
    store
        .connection
        .execute(
            "UPDATE recommendation_history SET created_at='2000-01-01T00:00:00Z' WHERE id=?1",
            [old],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE suggestions SET timestamp='2000-01-01T00:00:00Z'",
            [],
        )
        .unwrap();
    let new = store
        .record_recommendation(goal.id, "New", "https://example.com/new", "New result")
        .unwrap();
    store.purge_history(Some("2020-01-01T00:00:00Z")).unwrap();
    let history = store.recommendation_history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, new);
    assert!(!store
        .seen_suggestion(goal.id, "https://example.com/old")
        .unwrap());
}
#[test]
fn preview_context_uses_the_same_minimization_as_provider_requests() {
    let items: Vec<_> = (0..15)
        .map(|i| ActivitySnapshot {
            timestamp: String::new(),
            process_name: format!("app{i}"),
            window_title: "a".repeat(200),
            idle_seconds: 10,
            active_seconds: 60,
            ..Default::default()
        })
        .collect();
    let focus = focus::focus_context("Goal", &items);
    assert_eq!(focus["recent_activity"].as_array().unwrap().len(), 10);
    assert_eq!(focus["recent_activity"][0]["app"], "app5");
    assert_eq!(
        focus["recent_activity"][0]["title"].as_str().unwrap().len(),
        160
    );
    assert_eq!(
        focus::search_context("Goal", &items)["recent_window_titles"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}
#[tokio::test]
async fn connection_checks_use_get_without_private_context() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("x-api-key", "test-key"))
        .respond_with(
            crate::claude::test::reply(json!({"id":"test-model","display_name":"Test"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(check_connection(
        &reqwest::Client::new(),
        &server.uri(),
        "test-key",
        "anthropic",
        Some("test-model")
    )
    .await
    .unwrap()
    .contains("Connected"));
    assert!(server.received_requests().await.unwrap()[0].body.is_empty());
}
#[tokio::test]
async fn connection_errors_do_not_echo_provider_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_string("private key details"))
        .expect(1)
        .mount(&server)
        .await;
    let error = check_connection(
        &reqwest::Client::new(),
        &server.uri(),
        "test-key",
        "tavily",
        None,
    )
    .await
    .unwrap_err();
    assert!(error.contains("401"));
    assert!(!error.contains("private key details"));
}

#[tokio::test]
async fn model_probe_reports_missing_model_without_claiming_inference() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            crate::claude::test::reply(json!({"id":"other-model"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let result = probe_connection(
        &reqwest::Client::new(),
        &server.uri(),
        "fixture-key",
        "anthropic",
        Some("configured-model"),
    )
    .await
    .unwrap();
    assert_eq!(result.model_available, Some(false));
    assert!(result.message.contains("not available"));
    assert!(server.received_requests().await.unwrap()[0].body.is_empty());
}

#[tokio::test]
async fn tavily_probe_verifies_usage_without_sending_a_search_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(crate::claude::test::reply(json!({"key":{"usage":3}})))
        .expect(1)
        .mount(&server)
        .await;
    let result = probe_connection(
        &reqwest::Client::new(),
        &server.uri(),
        "fixture-key",
        "tavily",
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.model_available, None);
    assert!(result.message.contains("search access has not been tested"));
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].body.is_empty());
    assert!(!result.message.contains("fixture-key"));
}
