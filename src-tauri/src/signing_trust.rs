use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

/// The GitHub repository releases are published from; updates come only from it.
pub const RELEASE_REPOSITORY: &str = "KateNorth7/desktop_buddy";

#[derive(Deserialize)]
pub struct Trust {
    pub epoch: u32,
    pub signers: Vec<Signer>,
}
#[derive(Deserialize)]
pub struct Signer {
    pub id: String,
    pub pubkey: String,
}

fn decoded(value: &str) -> Result<String, String> {
    if value.len() > 4096 {
        return Err("Invalid signing metadata".into());
    }
    String::from_utf8(
        STANDARD
            .decode(value)
            .map_err(|_| "Invalid signing metadata")?,
    )
    .map_err(|_| "Invalid signing metadata".into())
}
fn key_id(text: &str) -> Result<[u8; 8], String> {
    let line = text.lines().nth(1).ok_or("Invalid signing metadata")?;
    let bytes = STANDARD
        .decode(line)
        .map_err(|_| "Invalid signing metadata")?;
    bytes
        .get(2..10)
        .ok_or("Invalid signing metadata")?
        .try_into()
        .map_err(|_| "Invalid signing metadata".into())
}
impl Trust {
    pub fn load() -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        return Self::parse(include_str!("../trusted-signers.macos.json"));
        #[cfg(not(target_os = "macos"))]
        Self::parse(include_str!("../trusted-signers.json"))
    }
    fn parse(json: &str) -> Result<Self, String> {
        let trust: Self =
            serde_json::from_str(json).map_err(|_| "Signing configuration is unavailable")?;
        if trust.epoch == 0 || trust.signers.is_empty() || trust.signers.len() > 16 {
            return Err("Invalid signing configuration".into());
        }
        let mut keys = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for signer in &trust.signers {
            let text = decoded(&signer.pubkey)?;
            PublicKey::decode(&text).map_err(|_| "Invalid trusted public key")?;
            if signer.id.is_empty() || !keys.insert(key_id(&text)?) || !names.insert(&signer.id) {
                return Err("Duplicate trusted signer".into());
            }
        }
        Ok(trust)
    }
    pub fn public_key_for(&self, signature: &str) -> Result<&str, String> {
        let text = decoded(signature)?;
        Signature::decode(&text).map_err(|_| "Invalid update signature")?;
        let id = key_id(&text)?;
        for signer in &self.signers {
            if key_id(&decoded(&signer.pubkey)?)? == id {
                return Ok(&signer.pubkey);
            }
        }
        Err("This update was signed by an unrecognized developer. Wait for a trusted transition release.".into())
    }
    pub fn endpoint(&self) -> String {
        let channel = if cfg!(target_os = "macos") {
            "updates-macos-epoch"
        } else {
            "updates-epoch"
        };
        format!(
            "https://github.com/{RELEASE_REPOSITORY}/releases/latest/download/{channel}-{}.json",
            self.epoch
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../test-data/signing.json")).unwrap()
    }
    fn trust() -> Trust {
        let data = fixture();
        Trust::parse(
            &serde_json::json!({"epoch":2,"signers":data["signers"].as_array().unwrap()[..2]})
                .to_string(),
        )
        .unwrap()
    }
    #[test]
    fn accepts_two_independent_signers_and_rejects_unknown_or_malformed_metadata() {
        let data = fixture();
        let trust = trust();
        for signer in data["signers"].as_array().unwrap().iter().take(2) {
            assert_eq!(
                trust
                    .public_key_for(signer["signature"].as_str().unwrap())
                    .unwrap(),
                signer["pubkey"].as_str().unwrap()
            );
        }
        assert!(trust
            .public_key_for(data["signers"][2]["signature"].as_str().unwrap())
            .is_err());
        for bad in ["", "not a signature", "ZHVtbXk="] {
            assert!(trust.public_key_for(bad).is_err());
        }
    }
    #[test]
    fn both_developer_signatures_verify_but_tampered_content_or_comments_do_not() {
        let data = fixture();
        for signer in data["signers"].as_array().unwrap().iter().take(2) {
            let key =
                PublicKey::decode(&decoded(signer["pubkey"].as_str().unwrap()).unwrap()).unwrap();
            let signature_text = decoded(signer["signature"].as_str().unwrap()).unwrap();
            let signature = Signature::decode(&signature_text).unwrap();
            assert!(key
                .verify(data["data"].as_str().unwrap().as_bytes(), &signature, false)
                .is_ok());
            assert!(key
                .verify(b"tampered installer", &signature, false)
                .is_err());
            let altered =
                Signature::decode(&signature_text.replace("version:0.13.0", "version:9.99.0"))
                    .unwrap();
            assert!(key
                .verify(data["data"].as_str().unwrap().as_bytes(), &altered, false)
                .is_err());
        }
    }
    #[test]
    fn rejects_duplicate_signers_and_pins_each_trust_epoch() {
        let data = fixture();
        assert!(Trust::parse(
            &serde_json::json!({"epoch":1,"signers":[data["signers"][0],data["signers"][0]]})
                .to_string()
        )
        .is_err());
        assert!(Trust::parse(r#"{"epoch":0,"signers":[]}"#).is_err());
        let name = if cfg!(target_os = "macos") {
            "updates-macos-epoch-2.json"
        } else {
            "updates-epoch-2.json"
        };
        assert!(trust().endpoint().ends_with(name));
        assert!(Trust::load()
            .unwrap()
            .signers
            .iter()
            .all(|s| !data["signers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|fixture| fixture["pubkey"].as_str() == Some(s.pubkey.as_str()))));
    }
}
