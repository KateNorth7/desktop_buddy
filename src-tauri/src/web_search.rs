//! Web search runs through Claude's server-side `web_search` tool on the same
//! connection as every other AI request. The results keep the internal
//! `{title, url, content}` shape the research pipeline validates and ranks.
use crate::{http, models::SearchResult};
use serde::Deserialize;
use serde_json::Value;
#[derive(Deserialize)]
struct Response {
    results: Vec<SearchResult>,
}
pub fn parse_results(value: Value) -> Result<Vec<SearchResult>, String> {
    let result: Response =
        serde_json::from_value(value).map_err(|_| "Web search returned invalid search results")?;
    Ok(result
        .results
        .into_iter()
        .filter(|r| {
            reqwest::Url::parse(&r.url)
                .map(|u| {
                    matches!(u.scheme(), "https" | "http")
                        && u.username().is_empty()
                        && u.password().is_none()
                        && u.host_str().is_some()
                })
                .unwrap_or(false)
        })
        .take(5)
        .collect())
}
pub async fn search(client: &reqwest::Client, query: &str) -> Result<Vec<SearchResult>, String> {
    let url = http::endpoint("ANTHROPIC_API_URL")?;
    let key = http::secret("ANTHROPIC_API_KEY")?;
    call(client, &url, &key, query).await
}
async fn call(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    let payload = crate::claude::web_search_payload(&http::secret("ANTHROPIC_MODEL_ID")?, query, 1);
    crate::claude::search_results(&http::post_json(client, url, key, &payload).await?)
        .and_then(parse_results)
}
pub async fn research(client: &reqwest::Client, query: &str) -> Result<Vec<SearchResult>, String> {
    research_call(
        client,
        &http::endpoint("ANTHROPIC_API_URL")?,
        &http::secret("ANTHROPIC_API_KEY")?,
        query,
    )
    .await
}
/// Research may run two searches for one query; every search is billed, so the
/// tool's own `max_uses` bounds it.
pub(crate) async fn research_call(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    let payload = crate::claude::web_search_payload(&http::secret("ANTHROPIC_MODEL_ID")?, query, 2);
    let results =
        crate::claude::search_results(&http::post_json(client, url, key, &payload).await?)
            .and_then(parse_results)?;
    Ok(results
        .into_iter()
        .map(|r| SearchResult {
            title: r.title.chars().take(180).collect(),
            url: r.url,
            content: r.content.chars().take(2400).collect(),
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{body_partial_json, header, method},
        Mock, MockServer, ResponseTemplate,
    };
    #[test]
    fn excludes_unsafe_links() {
        assert!(parse_results(
            json!({"results":[{"title":"Bad","url":"javascript:alert(1)","content":"x"}]})
        )
        .unwrap()
        .is_empty());
    }
    #[test]
    fn rejects_missing_results() {
        assert!(parse_results(json!({})).is_err());
    }
    #[tokio::test]
    async fn research_sends_only_the_query_with_a_bounded_search_tool_and_reports_invalid_keys_safely(
    ) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("x-api-key", "fixture"))
            .and(body_partial_json(json!({"tools":[{"type":"web_search_20260209","name":"web_search","max_uses":2,"allowed_callers":["direct"]}]})))
            .respond_with(|request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["messages"].as_array().unwrap().len(), 1);
                assert_eq!(body["messages"][0]["content"], "official application workflow");
                assert_eq!(body["output_config"]["effort"], "low");
                assert!(body["output_config"].get("format").is_none());
                crate::claude::test::search_reply(json!([{"title":"Official workflow","url":"https://example.com/workflow","content":"x".repeat(5000)}]))
            })
            .expect(1)
            .mount(&server)
            .await;
        let results = research_call(
            &reqwest::Client::new(),
            &server.uri(),
            "fixture",
            "official application workflow",
        )
        .await
        .unwrap();
        assert_eq!(results[0].title, "Official workflow");
        assert_eq!(results[0].content.len(), 2400);
        let invalid = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("secret-body-fixture"))
            .expect(1)
            .mount(&invalid)
            .await;
        let error = research_call(&reqwest::Client::new(), &invalid.uri(), "fixture", "query")
            .await
            .unwrap_err();
        assert!(!error.contains("secret-body-fixture"));
        assert!(!error.contains("fixture"));
    }
    #[tokio::test]
    async fn searches_and_normalizes_results() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({"tools":[{"name":"web_search","max_uses":1}]}),
            ))
            .respond_with(crate::claude::test::search_reply(
                json!([{"title":"Focus","url":"https://example.com","content":"Useful"}]),
            ))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            call(&reqwest::Client::new(), &server.uri(), "test", "focus")
                .await
                .unwrap()[0]
                .title,
            "Focus"
        );
    }
}
