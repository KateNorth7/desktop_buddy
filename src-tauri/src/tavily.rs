use crate::{http, models::SearchResult};
use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Deserialize)]
struct Response {
    results: Vec<SearchResult>,
}
pub fn parse_results(value: Value) -> Result<Vec<SearchResult>, String> {
    let result: Response =
        serde_json::from_value(value).map_err(|_| "Tavily returned invalid search results")?;
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
pub async fn tavily_search(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    let url = http::endpoint("TAVILY_API_URL")?;
    let key = http::secret("TAVILY_API_KEY")?;
    call(client, &url, &key, query).await
}
async fn call(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    parse_results(
        http::post_json(
            client,
            url,
            key,
            &json!({"query":query,"search_depth":"basic","max_results":5}),
        )
        .await?,
    )
}
pub async fn research(client: &reqwest::Client, query: &str) -> Result<Vec<SearchResult>, String> {
    research_call(
        client,
        &http::endpoint("TAVILY_API_URL")?,
        &http::secret("TAVILY_API_KEY")?,
        query,
    )
    .await
}
pub(crate) async fn research_call(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    let results = parse_results(http::post_json(client, url, key,
        &json!({"query":query,"search_depth":"advanced","topic":"general","max_results":5,"include_raw_content":false})).await?)?;
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
    use wiremock::{
        matchers::{body_partial_json, method},
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
    async fn research_uses_advanced_search_and_reports_invalid_keys_safely() {
        use wiremock::matchers::header;
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(header("authorization","Bearer fixture"))
            .and(body_partial_json(json!({"query":"official application workflow","search_depth":"advanced","topic":"general","max_results":5,"include_raw_content":false})))
            .respond_with(crate::claude::test::reply(json!({"results":[{"title":"Official workflow","url":"https://example.com/workflow","content":"x".repeat(5000)}]})))
            .expect(1).mount(&server).await;
        let results = research_call(
            &reqwest::Client::new(),
            &server.uri(),
            "fixture",
            "official application workflow",
        )
        .await
        .unwrap();
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
        Mock::given(method("POST")).and(body_partial_json(json!({"query":"focus"}))).respond_with(crate::claude::test::reply(json!({"results":[{"title":"Focus","url":"https://example.com","content":"Useful","score":0.9}]}))).expect(1).mount(&server).await;
        assert_eq!(
            call(&reqwest::Client::new(), &server.uri(), "test", "focus")
                .await
                .unwrap()[0]
                .title,
            "Focus"
        );
    }
}
