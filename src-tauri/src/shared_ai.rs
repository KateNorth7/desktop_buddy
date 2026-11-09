//! Built-in AI keeps provider credentials on Supabase. Only anonymous user sessions
//! are held locally; they cannot read tables or obtain provider/service-role keys.
use reqwest::{Method, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const SHARED_CREDENTIAL: &str = "sumi-built-in-connection";
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    #[default]
    BuiltIn,
    Personal,
}
#[cfg(not(test))]
static CONNECTION: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static CONNECTION: AtomicU64 = AtomicU64::new(1);
static SESSION: OnceLock<tokio::sync::Mutex<Option<Session>>> = OnceLock::new();
static SESSION_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicConfig {
    url: String,
    publishable_key: String,
}
fn config() -> Result<&'static PublicConfig, String> {
    static CONFIG: OnceLock<Result<PublicConfig, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let config: PublicConfig =
                serde_json::from_str(include_str!("../../server/supabase/public.json"))
                    .map_err(|_| "Built-in AI configuration unavailable")?;
            if config.url != "https://qrspcvglmmrputcltgxr.supabase.co"
                || config.publishable_key.is_empty()
            {
                return Err("Invalid built-in AI configuration".into());
            }
            Ok(config)
        })
        .as_ref()
        .map_err(Clone::clone)
}
pub(crate) fn mode() -> ConnectionMode {
    if CONNECTION.load(Ordering::SeqCst) & 1 == 0 {
        ConnectionMode::BuiltIn
    } else {
        ConnectionMode::Personal
    }
}
pub(crate) fn configured() -> bool {
    config().is_ok()
}
#[cfg(not(test))]
fn command_line_mode() -> Option<ConnectionMode> {
    if !std::env::args().any(|arg| arg.starts_with("--check-")) {
        return None;
    }
    match std::env::var("BUDDY_AI_CONNECTION_MODE").as_deref() {
        Ok("personal") => Some(ConnectionMode::Personal),
        Ok("built_in") => Some(ConnectionMode::BuiltIn),
        _ => None,
    }
}
#[cfg(not(test))]
pub(crate) fn initialize_cli() {
    if let Some(mode) = command_line_mode() {
        activate(mode);
    }
}
#[cfg(not(test))]
pub(crate) fn initialize(storage: &crate::storage::Storage) -> Result<(), String> {
    let preferences: crate::core::CorePreferences = storage
        .read_setting("core_preferences")?
        .unwrap_or_default();
    let mode = command_line_mode().unwrap_or(preferences.ai_connection_mode);
    activate(mode);
    if let Some(path) = storage.connection.path().filter(|path| !path.is_empty()) {
        *SESSION_PATH
            .lock()
            .map_err(|_| "Shared session storage unavailable")? = std::path::Path::new(path)
            .parent()
            .map(|dir| dir.join("shared-ai-session.json"));
    }
    Ok(())
}
#[cfg(not(test))]
pub(crate) fn activate(mode: ConnectionMode) {
    let bit = u64::from(mode == ConnectionMode::Personal);
    let _ = CONNECTION.try_update(Ordering::SeqCst, Ordering::SeqCst, |previous| {
        ((previous & 1) != bit).then_some(((previous & !1).wrapping_add(2)) | bit)
    });
}
pub(crate) fn endpoint(provider: &str) -> Result<String, String> {
    let suffix = match provider {
        "ANTHROPIC_API_URL" => "/anthropic/v1/messages",
        _ => return Err("Unknown built-in service".into()),
    };
    Ok(format!("{}/functions/v1/buddy-ai{suffix}", config()?.url))
}
fn route_allowed(config: &PublicConfig, mode: ConnectionMode, url: &str, key: &str) -> bool {
    let root = format!("{}/functions/v1/buddy-ai", config.url);
    if mode == ConnectionMode::Personal {
        return key != SHARED_CREDENTIAL && !url.starts_with(&root);
    }
    key == SHARED_CREDENTIAL
        && [
            "/anthropic/v1/messages".to_string(),
            format!(
                "/anthropic/v1/models/{}",
                crate::http::DEFAULT_PRIMARY_MODEL
            ),
        ]
        .iter()
        .any(|suffix| url == format!("{root}{suffix}"))
}
/// How a personal credential is presented to the provider. Built-in requests
/// always carry the Supabase session instead; the gateway adds provider headers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Auth {
    /// Claude API: `x-api-key` plus the pinned `anthropic-version`.
    Claude,
    /// Other bearer-token services.
    Bearer,
}
#[derive(Clone, Copy)]
pub(crate) struct Guard(u64);
impl Guard {
    pub(crate) fn check(&self) -> Result<(), String> {
        if CONNECTION.load(Ordering::SeqCst) == self.0 {
            Ok(())
        } else {
            Err("AI connection changed. Try again using the selected connection.".into())
        }
    }
    pub(crate) fn built_in(&self) -> bool {
        self.0 & 1 == 0
    }
}
pub(crate) async fn prepare(
    client: &reqwest::Client,
    method: Method,
    url: &str,
    key: &str,
    auth: Auth,
) -> Result<(RequestBuilder, Guard), String> {
    let guard = Guard(CONNECTION.load(Ordering::SeqCst));
    let mode = if guard.built_in() {
        ConnectionMode::BuiltIn
    } else {
        ConnectionMode::Personal
    };
    if !route_allowed(config()?, mode, url, key) {
        return Err(
            "AI connection changed or an unsupported built-in operation was requested".into(),
        );
    }
    let request = client.request(method, url);
    let request = if mode == ConnectionMode::BuiltIn {
        request
            .bearer_auth(access_token(client).await?)
            .header("apikey", &config()?.publishable_key)
    } else {
        match auth {
            Auth::Claude => request
                .header("x-api-key", key)
                .header("anthropic-version", crate::claude::API_VERSION)
                .header("anthropic-beta", crate::claude::BETA_FEATURES),
            Auth::Bearer => request.bearer_auth(key),
        }
    };
    guard.check()?;
    Ok((request, guard))
}
#[derive(Serialize, Deserialize, Clone)]
struct Session {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn read_session() -> Option<Session> {
    use std::io::Read;
    let path = SESSION_PATH.lock().ok()?.clone()?;
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 32768 {
        return None;
    }
    let mut raw = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(32769)
        .read_to_end(&mut raw)
        .ok()?;
    serde_json::from_slice(&raw).ok()
}
fn store_session(session: &Session) -> Result<(), String> {
    use std::io::Write;
    let Some(path) = SESSION_PATH
        .lock()
        .map_err(|_| "Session storage unavailable")?
        .clone()
    else {
        return Ok(());
    };
    let temp = path.with_extension(format!("{}.tmp", now()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temp)
            .map_err(|_| "Session storage unavailable")?;
        file.write_all(
            &serde_json::to_vec(session).map_err(|_| "Session serialization unavailable")?,
        )
        .map_err(|_| "Session storage unavailable")?;
        drop(file);
        // Windows does not replace an existing file with rename. These are only
        // anonymous session credentials, never provider keys or user history.
        #[cfg(windows)]
        if path.exists() {
            std::fs::remove_file(&path).map_err(|_| "Session storage unavailable")?;
        }
        std::fs::rename(&temp, &path).map_err(|_| "Session storage unavailable")
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result.map_err(str::to_owned)
}
async fn access_token(client: &reqwest::Client) -> Result<String, String> {
    let mut current = SESSION
        .get_or_init(|| tokio::sync::Mutex::new(None))
        .lock()
        .await;
    if current.is_none() {
        *current = read_session();
    }
    if let Some(session) = &*current {
        if session.expires_at > now() + 60 {
            return Ok(session.access_token.clone());
        }
    }
    let session = authenticate(client, config()?, current.as_ref()).await?;
    let token = session.access_token.clone();
    let _ = store_session(&session);
    *current = Some(session);
    Ok(token)
}
async fn authenticate(
    client: &reqwest::Client,
    config: &PublicConfig,
    previous: Option<&Session>,
) -> Result<Session, String> {
    let (path, body) = match previous {
        Some(session) => (
            "/auth/v1/token?grant_type=refresh_token",
            serde_json::json!({"refresh_token":session.refresh_token}),
        ),
        None => ("/auth/v1/signup", serde_json::json!({"data":{}})),
    };
    let mut response = client
        .post(format!("{}{path}", config.url))
        .header("apikey", &config.publishable_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| "Built-in AI could not connect. Check your network.")?;
    // A revoked/expired anonymous refresh session can be renewed invisibly.
    // Never reset an identity in response to allowance or network failures.
    if previous.is_some() && matches!(response.status().as_u16(), 400 | 401) {
        response = client
            .post(format!("{}/auth/v1/signup", config.url))
            .header("apikey", &config.publishable_key)
            .json(&serde_json::json!({"data":{}}))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| "Built-in AI could not renew its session")?;
    }
    if !response.status().is_success() {
        return Err("Built-in AI could not start or refresh its session. Try again later or use your own API keys.".into());
    }
    let raw = response
        .bytes()
        .await
        .map_err(|_| "Invalid built-in session response")?;
    if raw.len() > 32768 {
        return Err("Invalid built-in session response".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| "Invalid built-in session response")?;
    let session = Session {
        access_token: value["access_token"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() < 8192)
            .ok_or("Built-in session token missing")?
            .into(),
        refresh_token: value["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() < 4096)
            .ok_or("Built-in session refresh unavailable")?
            .into(),
        expires_at: value["expires_at"]
            .as_u64()
            .unwrap_or_else(|| now() + value["expires_in"].as_u64().unwrap_or(3600).min(86400)),
    };
    if value["user"]["is_anonymous"] != true {
        return Err("Unexpected built-in session identity".into());
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_json, header, method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    fn session_response() -> serde_json::Value {
        serde_json::json!({"access_token":"session-access-fixture","refresh_token":"session-refresh-fixture",
            "expires_in":3600,"user":{"is_anonymous":true}})
    }
    #[tokio::test]
    async fn automatic_signup_uses_only_the_public_client_key_and_no_user_details() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/v1/signup"))
            .and(header("apikey", "public-client-fixture"))
            .and(body_json(serde_json::json!({"data":{}})))
            .respond_with(crate::claude::test::reply(session_response()))
            .expect(1)
            .mount(&server)
            .await;
        let config = PublicConfig {
            url: server.uri(),
            publishable_key: "public-client-fixture".into(),
        };
        let session = authenticate(&reqwest::Client::new(), &config, None)
            .await
            .unwrap();
        assert_eq!(session.access_token, "session-access-fixture");
        assert!(session.expires_at > now() + 3500);
    }
    #[tokio::test]
    async fn returning_installations_refresh_the_existing_session_instead_of_signing_up_again() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/v1/token"))
            .and(query_param("grant_type", "refresh_token"))
            .and(body_json(
                serde_json::json!({"refresh_token":"previous-refresh"}),
            ))
            .respond_with(crate::claude::test::reply(session_response()))
            .expect(1)
            .mount(&server)
            .await;
        let config = PublicConfig {
            url: server.uri(),
            publishable_key: "public-client-fixture".into(),
        };
        let previous = Session {
            access_token: "expired".into(),
            refresh_token: "previous-refresh".into(),
            expires_at: 0,
        };
        assert!(
            authenticate(&reqwest::Client::new(), &config, Some(&previous))
                .await
                .is_ok()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn allowance_failures_do_not_create_new_identities_or_repeat_authentication() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/v1/token"))
            .respond_with(ResponseTemplate::new(429))
            .expect(1)
            .mount(&server)
            .await;
        let config = PublicConfig {
            url: server.uri(),
            publishable_key: "public-client-fixture".into(),
        };
        let previous = Session {
            access_token: "expired".into(),
            refresh_token: "previous-refresh".into(),
            expires_at: 0,
        };
        assert!(
            authenticate(&reqwest::Client::new(), &config, Some(&previous))
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    #[test]
    fn saved_personal_connection_remains_personal_and_keeps_provider_keys_out_of_preferences() {
        let storage = crate::storage::Storage::open(std::path::Path::new(":memory:")).unwrap();
        let preferences = crate::core::CorePreferences {
            ai_connection_mode: ConnectionMode::Personal,
            ..Default::default()
        };
        storage
            .write_setting("core_preferences", &preferences)
            .unwrap();
        let loaded: crate::core::CorePreferences =
            storage.read_setting("core_preferences").unwrap().unwrap();
        assert_eq!(loaded.ai_connection_mode, ConnectionMode::Personal);
        let raw = serde_json::to_string(&loaded).unwrap();
        assert!(!raw.contains("api_key") && !raw.contains("access_token"));
    }
    #[test]
    fn built_in_is_the_default_and_personal_choice_survives_serialization() {
        assert_eq!(ConnectionMode::default(), ConnectionMode::BuiltIn);
        assert_eq!(
            serde_json::from_str::<ConnectionMode>("\"personal\"").unwrap(),
            ConnectionMode::Personal
        );
        let old: crate::core::CorePreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(old.ai_connection_mode, ConnectionMode::BuiltIn);
    }
    #[test]
    fn connection_modes_never_mix_credentials_or_allow_arbitrary_shared_destinations() {
        let config = config().unwrap();
        let url = endpoint("ANTHROPIC_API_URL").unwrap();
        assert!(route_allowed(
            config,
            ConnectionMode::BuiltIn,
            &url,
            SHARED_CREDENTIAL
        ));
        assert!(!route_allowed(
            config,
            ConnectionMode::BuiltIn,
            &url,
            "personal-key"
        ));
        assert!(!route_allowed(
            config,
            ConnectionMode::Personal,
            &url,
            SHARED_CREDENTIAL
        ));
        assert!(!route_allowed(
            config,
            ConnectionMode::BuiltIn,
            "https://attacker.invalid",
            SHARED_CREDENTIAL
        ));
        assert!(!route_allowed(
            config,
            ConnectionMode::BuiltIn,
            &(url + "?redirect=unsafe"),
            SHARED_CREDENTIAL
        ));
        assert!(route_allowed(
            config,
            ConnectionMode::Personal,
            "https://api.anthropic.com/v1/messages",
            "personal-key"
        ));
    }
    #[test]
    fn public_client_configuration_is_not_a_service_role_or_provider_secret() {
        use base64::Engine;
        let config = config().unwrap();
        let payload = config.publishable_key.split('.').nth(1).unwrap();
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(claims["role"], "anon");
    }
}
