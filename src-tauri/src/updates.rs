use serde::Serialize;
use std::time::Duration;
use tauri::{ipc::Channel, Manager, State, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct UpdateState(pub tokio::sync::Mutex<Option<Update>>);

#[derive(Clone, Serialize)]
pub struct UpdateInfo {
    pub status: &'static str,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct UpdateProgress {
    pub phase: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
}

fn workspace_only(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Open the workspace to manage updates".into())
    }
}

fn validate_download(url: &reqwest::Url) -> Result<(), String> {
    validate_platform_download(
        url,
        if cfg!(target_os = "macos") {
            ".app.tar.gz"
        } else {
            ".exe"
        },
    )
}
fn validate_platform_download(url: &reqwest::Url, extension: &str) -> Result<(), String> {
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !url.path().starts_with(&format!(
            "/{}/releases/download/",
            crate::signing_trust::RELEASE_REPOSITORY
        ))
        || !url.path().ends_with(extension)
    {
        return Err("The update does not match this platform or sumi's repository".into());
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn installation_argument(executable: &std::path::Path) -> Result<String, String> {
    if !executable.is_absolute() {
        return Err("The installed application path must be absolute".into());
    }
    let directory = executable
        .parent()
        .ok_or("The installation folder is unavailable")?;
    let directory = directory
        .to_str()
        .ok_or("The installation folder is not valid Unicode")?;
    if directory.contains(['"', '\n', '\r']) {
        return Err("The installation folder is invalid".into());
    }
    // NSIS requires /D to be last and unquoted, including paths with spaces.
    Ok(format!("/D={directory}"))
}

#[tauri::command(async)]
pub async fn check_app_update(
    window: WebviewWindow,
    state: State<'_, UpdateState>,
) -> Result<UpdateInfo, String> {
    workspace_only(&window)?;
    let mut pending = state
        .0
        .try_lock()
        .map_err(|_| "Another update action is running")?;
    *pending = None;
    let app = window.app_handle();
    let trust = crate::signing_trust::Trust::load()?;
    let mut info = UpdateInfo {
        status: "current",
        current_version: app.package_info().version.to_string(),
        version: None,
        notes: None,
    };
    // A new repository has no updater manifest yet. This is distinct from an
    // unreachable server, invalid manifest, or a signed newer release.
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Could not start the update check")?
        .get(trust.endpoint())
        .header("User-Agent", "sumi-Updater")
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Check your connection and try again.")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        info.status = "unpublished";
        return Ok(info);
    }
    if !response.status().is_success() {
        return Err("GitHub update service is unavailable. Try again later.".into());
    }
    let manifest: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid update manifest")?;
    let platform = if cfg!(target_os = "macos") {
        format!("darwin-{}", std::env::consts::ARCH)
    } else {
        "windows-x86_64".into()
    };
    let signature_pointer = format!("/platforms/{platform}/signature");
    if cfg!(target_os = "macos") && manifest.pointer(&signature_pointer).is_none() {
        info.status = "unpublished";
        return Ok(info);
    }
    let signature = manifest
        .pointer(&signature_pointer)
        .and_then(|s| s.as_str())
        .ok_or("The update has no signature for this platform")?;
    let pubkey = trust.public_key_for(signature)?;
    let builder = app
        .updater_builder()
        .endpoints(vec![trust
            .endpoint()
            .parse()
            .map_err(|_| "Invalid update endpoint")?])
        .map_err(|_| "Invalid update endpoint")?
        .pubkey(pubkey)
        .timeout(Duration::from_secs(15));
    #[cfg(windows)]
    let builder = builder.installer_arg(installation_argument(
        &std::env::current_exe().map_err(|_| "Could not locate the installed application")?,
    )?);
    let updater = builder
        .build()
        .map_err(|_| "Update configuration is unavailable")?;
    if let Some(mut update) = updater
        .check()
        .await
        .map_err(|_| "Could not validate the published update. Try again later.")?
    {
        // A manifest can change between requests. Never install with a mismatched key.
        if trust.public_key_for(&update.signature)? != pubkey {
            return Err("The release changed. Check for updates again.".into());
        }
        validate_download(&update.download_url)?;
        update.timeout = Some(Duration::from_secs(180));
        info.status = "available";
        info.version = Some(update.version.clone());
        info.notes = update.body.as_ref().map(|s| s.chars().take(8000).collect());
        *pending = Some(update);
    }
    Ok(info)
}

#[tauri::command(async)]
pub async fn install_app_update(
    version: String,
    progress: Channel<UpdateProgress>,
    window: WebviewWindow,
    state: State<'_, UpdateState>,
) -> Result<(), String> {
    workspace_only(&window)?;
    if cfg!(debug_assertions) {
        return Err("Install updates from the installed desktop app".into());
    }
    let mut pending = state
        .0
        .try_lock()
        .map_err(|_| "Another update action is running")?;
    let update = pending
        .as_ref()
        .ok_or("Check for updates before installing")?;
    if update.version != version {
        return Err("The available version changed. Check for updates again.".into());
    }
    validate_download(&update.download_url)?;
    let mut downloaded = 0u64;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let _ = progress.send(UpdateProgress {
                    phase: "downloading",
                    downloaded,
                    total,
                });
            },
            || {},
        )
        .await
        .map_err(|_| {
            "Update download or signature verification failed. Your installed version is unchanged."
        })?;
    // Download verifies the artifact before stopping the session or launching it.
    {
        let app_state = window.state::<crate::commands::AppState>();
        let mut inner = app_state
            .inner
            .lock()
            .map_err(|_| "Application state unavailable")?;
        crate::buddy::remember_position(window.app_handle(), &mut inner)?;
    }
    let _ = progress.send(UpdateProgress {
        phase: "installing",
        downloaded,
        total: Some(bytes.len() as u64),
    });
    update
        .install(bytes)
        .map_err(|_| "Could not install the update. Please try again.")?;
    *pending = None;
    #[cfg(target_os = "macos")]
    window.app_handle().restart();
    #[cfg(not(target_os = "macos"))]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pins_updates_to_the_running_installation_and_rejects_unsafe_paths() {
        let directory = std::env::temp_dir().join("sumi install with spaces");
        assert_eq!(
            installation_argument(&directory.join("desktop-buddy.exe")).unwrap(),
            format!("/D={}", directory.display())
        );
        assert!(installation_argument(std::path::Path::new("relative/buddy.exe")).is_err());
        assert!(
            installation_argument(&std::env::temp_dir().join("bad\nfolder").join("buddy.exe"))
                .is_err()
        );
    }
    #[test]
    fn accepts_only_https_installers_from_the_pinned_repository() {
        let valid = "https://github.com/KateNorth7/desktop_buddy/releases/download/v0.11.0/Desktop.Buddy_0.11.0_x64-setup.exe";
        assert!(validate_platform_download(&valid.parse().unwrap(), ".exe").is_ok());
        let mac = valid.replace("_x64-setup.exe", "_aarch64.app.tar.gz");
        assert!(validate_platform_download(&mac.parse().unwrap(), ".app.tar.gz").is_ok());
        assert!(validate_platform_download(&mac.parse().unwrap(), ".exe").is_err());
        assert!(validate_platform_download(&valid.parse().unwrap(), ".app.tar.gz").is_err());
        for invalid in [
            valid.replace("https:", "http:"),
            valid.replace("KateNorth7", "other"),
            valid.replace("KateNorth7", "ArtmarketVM"),
            valid.replace("github.com", "github.com.evil.example"),
            valid.replace(".exe", ".zip"),
            valid.replace("github.com", "user:password@github.com"),
        ] {
            assert!(validate_platform_download(&invalid.parse().unwrap(), ".exe").is_err());
        }
    }
}
