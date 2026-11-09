//! Runtime-only provider secrets. Never return saved credentials to the webview.
pub fn provider_name(provider: &str) -> Result<&'static str, String> {
    match provider {
        "anthropic" => Ok("ANTHROPIC_API_KEY"),
        _ => Err("Unknown provider".into()),
    }
}

pub fn validate_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    if key.is_empty() || key.len() > 2048 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("Enter an API key of 1–2048 characters without spaces".into());
    }
    Ok(key)
}

#[cfg(windows)]
mod vault {
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{Foundation::ERROR_NOT_FOUND, Security::Credentials::*},
    };
    fn target(name: &str) -> Vec<u16> {
        format!("DesktopBuddy/{name}")
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }
    pub fn read(name: &str) -> Result<Option<String>, String> {
        let target = target(name);
        let mut credential = std::ptr::null_mut();
        unsafe {
            if let Err(error) = CredReadW(
                PCWSTR(target.as_ptr()),
                CRED_TYPE_GENERIC,
                None,
                &mut credential,
            ) {
                if error.code() == ERROR_NOT_FOUND.to_hresult() {
                    return Ok(None);
                }
                return Err("Windows Credential Manager could not read the API key".into());
            }
            let entry = &*credential;
            let result = if entry.CredentialBlobSize == 0 {
                Ok(String::new())
            } else {
                String::from_utf8(
                    std::slice::from_raw_parts(
                        entry.CredentialBlob,
                        entry.CredentialBlobSize as usize,
                    )
                    .to_vec(),
                )
            };
            CredFree(credential.cast());
            result
                .map(Some)
                .map_err(|_| "Saved credential is invalid; replace it in Settings".into())
        }
    }
    pub fn write(name: &str, key: Option<&str>) -> Result<(), String> {
        let mut target = target(name);
        unsafe {
            if let Some(key) = key {
                let mut bytes = key.as_bytes().to_vec();
                let credential = CREDENTIALW {
                    Type: CRED_TYPE_GENERIC,
                    TargetName: PWSTR(target.as_mut_ptr()),
                    CredentialBlobSize: bytes.len() as u32,
                    CredentialBlob: bytes.as_mut_ptr(),
                    Persist: CRED_PERSIST_LOCAL_MACHINE,
                    ..Default::default()
                };
                CredWriteW(&credential, 0).map_err(|_| {
                    "Windows Credential Manager could not save the API key".to_string()
                })
            } else {
                match CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) {
                    Ok(()) => Ok(()),
                    Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
                    Err(_) => Err("Windows Credential Manager could not remove the API key".into()),
                }
            }
        }
    }
    pub fn exists(name: &str) -> Result<bool, String> {
        read(name).map(|key| key.is_some())
    }
}
#[cfg(target_os = "macos")]
mod vault {
    use std::{
        collections::HashMap,
        sync::{Mutex, OnceLock},
    };
    type Entry = Result<Option<String>, String>;
    /// The pinned, immutable credential helper accepts only its two original
    /// Keychain account names. The Claude key therefore occupies the slot that
    /// previously held the Nebius key; renaming it requires a reviewed helper
    /// rebuild (see `native/credential-helper/README.md`).
    fn account(name: &str) -> &str {
        if name == "ANTHROPIC_API_KEY" {
            "NEBIUS_API_KEY"
        } else {
            name
        }
    }
    pub fn exists(name: &str) -> Result<bool, String> {
        crate::macos::has_key(account(name))
    }
    fn cache() -> &'static Mutex<HashMap<String, Entry>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }
    pub fn read(name: &str) -> Result<Option<String>, String> {
        // Serialize first reads: background tasks must not open concurrent authorization dialogs.
        let mut cache = cache()
            .lock()
            .map_err(|_| "Credential access unavailable")?;
        if let Some(result) = cache.get(name) {
            return result.clone();
        }
        let result = crate::macos::read_key(account(name));
        cache.insert(name.into(), result.clone());
        result
    }
    pub fn retry_failed(name: &str) {
        if let Ok(mut entries) = cache().lock() {
            if entries.get(name).is_some_and(|result| result.is_err()) {
                entries.remove(name);
            }
        }
    }
    pub fn write(name: &str, key: Option<&str>) -> Result<(), String> {
        let mut cache = cache()
            .lock()
            .map_err(|_| "Credential access unavailable")?;
        crate::macos::write_key(account(name), key)?;
        cache.insert(name.into(), Ok(key.map(str::to_string)));
        Ok(())
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
mod vault {
    pub fn read(_: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    pub fn write(_: &str, _: Option<&str>) -> Result<(), String> {
        Err("Saving API keys is supported on Windows only".into())
    }
    pub fn exists(_: &str) -> Result<bool, String> {
        Ok(false)
    }
}
pub use vault::{exists, read, write};
pub fn retry_failed_read(name: &str) {
    #[cfg(target_os = "macos")]
    vault::retry_failed(name);
    #[cfg(not(target_os = "macos"))]
    let _ = name;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_known_providers() {
        assert_eq!(provider_name("anthropic").unwrap(), "ANTHROPIC_API_KEY");
        assert!(provider_name("arbitrary-secret").is_err());
    }
    #[test]
    fn validates_without_echoing_secrets() {
        assert_eq!(validate_key(" test-key ").unwrap(), "test-key");
        for key in ["", "a b", "a\nb", "é"] {
            assert!(validate_key(key).is_err());
        }
        assert!(validate_key(&"x".repeat(2049)).is_err());
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "Writes an isolated test credential to Windows Credential Manager"]
    fn windows_vault_round_trip() {
        let name = format!(
            "test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        );
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = write(&self.0, None);
            }
        }
        let _cleanup = Cleanup(name.clone());
        assert!(read(&name).unwrap().is_none());
        write(&name, Some("test-key-only")).unwrap();
        assert_eq!(read(&name).unwrap().as_deref(), Some("test-key-only"));
        write(&name, Some("replacement-test-key")).unwrap();
        assert_eq!(
            read(&name).unwrap().as_deref(),
            Some("replacement-test-key")
        );
        write(&name, None).unwrap();
        assert!(read(&name).unwrap().is_none());
        write(&name, None).unwrap();
    }
}
