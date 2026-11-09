use crate::{
    collector,
    commands::{AppState, Inner},
    http, focus,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri::{AppHandle, State};
#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;

pub fn validate_retention(days: u32) -> Result<(), String> {
    if [0, 7, 30, 90].contains(&days) {
        Ok(())
    } else {
        Err("Choose 7, 30, 90 days, or keep until manually deleted".into())
    }
}
fn invalidate(inner: &mut Inner) {
    inner.mark_consent_change();
    inner.buddy.clear();
    inner.buddy.foreground = None;
    inner.collector = collector::create(inner.status.demo);
    inner.usage = Default::default();
    inner.activity_state.stop(false);
    inner.last_error = None;
}
pub fn cleanup_due(inner: &mut Inner) -> Result<(), String> {
    if inner.last_cleanup.elapsed() < Duration::from_secs(3600) {
        return Ok(());
    }
    if inner.retention_days > 0 {
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::days(inner.retention_days as i64)).to_rfc3339();
        inner.storage.purge_history(Some(&cutoff))?;
        invalidate(inner);
    }
    inner.last_cleanup = Instant::now();
    Ok(())
}
#[tauri::command(async)]
pub fn set_retention(days: u32, app: AppHandle, state: State<AppState>) -> Result<(), String> {
    validate_retention(days)?;
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if days > 0 {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        inner.storage.purge_history(Some(&cutoff))?;
    }
    inner.storage.write_setting("retention_days", &days)?;
    inner.retention_days = days;
    inner.last_cleanup = Instant::now();
    invalidate(&mut inner);
    crate::buddy::sync(&app, &mut inner)
}
#[tauri::command(async)]
pub fn clear_local_history(
    confirmed: bool,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    if !confirmed {
        return Err("Confirm permanent deletion of local history first".into());
    }
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.storage.purge_history(None)?;
    inner.status.tracking = false;
    inner.storage.write_setting("tracking_requested", &false)?;
    invalidate(&mut inner);
    crate::frontend_events::changed(&app, crate::frontend_events::ALL, true);
    crate::buddy::sync(&app, &mut inner)
}
#[tauri::command(async)]
pub fn get_privacy_preview(state: State<AppState>) -> Result<Value, String> {
    let inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    let goal = inner.storage.goal()?;
    let activity = match &goal {
        Some(g) => inner.storage.recent(g.id)?,
        None => vec![],
    };
    let activity: Vec<_> = activity
        .into_iter()
        .filter(|a| !crate::attention::excluded(&inner.buddy.view.preferences, &a.process_name))
        .collect();
    let memory = goal
        .as_ref()
        .map(|g| inner.storage.recommendation_memory(g.id))
        .transpose()?
        .unwrap_or_else(|| json!({"ratings":[],"recently_offered_titles":[]}));
    let goal = goal
        .as_ref()
        .map(|g| inner.storage.goal_context(g))
        .transpose()?
        .unwrap_or_default();
    Ok(
        json!({"focus_check":focus::focus_context(&goal,&activity),"proactive_search_planning":focus::recommendation_search_context(&goal,&activity,&memory),
        "anthropic_destination":destination("ANTHROPIC_API_URL")?,"tavily_destination":destination("TAVILY_API_URL")?,
        "note":"Local preview of user context only; no request was sent. Context may change before the next request. Query planning and resource selection use up to ten rated titles and ten recently offered titles for this goal. Resource selection additionally sends up to five candidate titles, URLs and snippets to Claude. Tavily receives only your typed or generated search query, not the raw activity list or rating history."}),
    )
}
fn destination(name: &str) -> Result<String, String> {
    Ok(reqwest::Url::parse(&http::endpoint(name)?)
        .map_err(|_| "Invalid service endpoint")?
        .origin()
        .ascii_serialization())
}

/// Derives the provider's health route from its generation endpoint. The Claude
/// check reads one model object, so pagination of the catalog cannot hide it.
pub fn test_url(endpoint: &str, provider: &str, model: Option<&str>) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(endpoint).map_err(|_| "Invalid service endpoint")?;
    let (suffix, target) = match provider {
        "anthropic" => {
            let model = model.filter(|m| !m.is_empty() && m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')).ok_or("Configure a Claude model before checking the connection")?;
            ("/messages", format!("/models/{model}"))
        }
        "tavily" => ("/search", "/usage".to_string()),
        _ => return Err("Unknown provider".into()),
    };
    let path = url
        .path()
        .trim_end_matches('/')
        .strip_suffix(suffix)
        .ok_or("Connection checks require a standard provider endpoint path")?
        .to_string()
        + &target;
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}
pub struct ConnectionProbe {
    pub message: String,
    pub model_available: Option<bool>,
}
pub async fn probe_connection(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    provider: &str,
    model: Option<&str>,
) -> Result<ConnectionProbe, String> {
    let auth = if provider == "anthropic" {
        crate::shared_ai::Auth::Claude
    } else {
        crate::shared_ai::Auth::Bearer
    };
    let (request, guard) =
        crate::shared_ai::prepare(client, reqwest::Method::GET, url, key, auth).await?;
    let response = request
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| "Connection failed. Check your network and endpoint settings.")?;
    guard.check()?;
    if provider == "anthropic" && response.status().as_u16() == 404 {
        // The key authenticated; only the requested model object is missing.
        return Ok(ConnectionProbe {
            model_available: Some(false),
            message: "Connected, but the configured model is not available to this key. Update ANTHROPIC_MODEL_ID or check model access. No focus request was sent.".into(),
        });
    }
    if !response.status().is_success() {
        return Err(http::status_error(response.status().as_u16()));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| "Provider returned an invalid connection-check response")?;
    guard.check()?;
    match provider {
        "anthropic" => {
            let id = data.get("id").and_then(Value::as_str).ok_or("Claude returned an invalid model response")?;
            let available = Some(id) == model;
            Ok(ConnectionProbe { model_available: Some(available), message: if available {
                "Connected. The configured model is available. Generation, billing, and structured-output support have not been tested."
            } else {
                "Connected, but the configured model is not available to this key. Update ANTHROPIC_MODEL_ID or check model access. No focus request was sent."
            }.into() })
        }
        "tavily" if data.get("key").is_some_and(Value::is_object) => Ok(ConnectionProbe {
            model_available: None,
            message: "Connected. The key can access usage information. No search query was sent; search access has not been tested.".into(),
        }),
        _ => Err("Provider returned an invalid connection-check response".into()),
    }
}
#[cfg(test)]
pub async fn check_connection(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    provider: &str,
    model: Option<&str>,
) -> Result<String, String> {
    probe_connection(client, url, key, provider, model)
        .await
        .map(|r| r.message)
}
#[tauri::command(async)]
pub async fn test_provider_connection(
    window: tauri::WebviewWindow,
    provider: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if window.label() != "main" {
        return Err("Open Settings in the main window".into());
    }
    let credential = crate::credentials::provider_name(&provider)?;
    crate::credentials::retry_failed_read(credential);
    let revision = {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| "Application state unavailable")?;
        crate::engine_observability::begin_connection_check(&mut inner, &provider)?;
        inner.last_consent_change
    };
    let result = async {
        let (endpoint, key, model) = if provider == "anthropic" {
            (
                http::endpoint("ANTHROPIC_API_URL")?,
                http::secret("ANTHROPIC_API_KEY")?,
                Some(http::secret("ANTHROPIC_MODEL_ID")?),
            )
        } else {
            (
                http::endpoint("TAVILY_API_URL")?,
                http::secret("TAVILY_API_KEY")?,
                None,
            )
        };
        let url = test_url(&endpoint, &provider, model.as_deref())?;
        probe_connection(
            &state.client,
            url.as_str(),
            &key,
            &provider,
            model.as_deref(),
        )
        .await
    }
    .await;
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    crate::engine_observability::finish_connection_check(
        &mut inner,
        &provider,
        revision,
        result.is_ok(),
        result.as_ref().ok().and_then(|r| r.model_available),
    );
    crate::frontend_events::changed(
        window.app_handle(),
        &[
            crate::frontend_events::Resource::Services,
            crate::frontend_events::Resource::Engine,
        ],
        false,
    );
    if inner.last_consent_change != revision {
        return Err("Connection settings changed during the check. Try again.".into());
    }
    result.map(|r| r.message)
}
