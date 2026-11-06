//! Encrypted review-only evidence. The key lives in the platform credential store,
//! never beside the database. Tests use an isolated synthetic key.
use super::*;
#[cfg(not(test))]
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};

const MAX_REVIEW_BYTES: i64 = 64 * 1024 * 1024;
/// Image descriptions a single review period may request; beyond this, blocks are
/// reviewed on their text evidence.
const MAX_DESCRIBED_IMAGES: i64 = 24;
#[cfg(any(windows, test))]
const KEY_NAME: &str = "BUDDY_ACTIVITY_EVIDENCE";

#[cfg(not(test))]
fn vault(secret: Option<&str>) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        if let Some(secret) = secret {
            crate::credentials::write(KEY_NAME, Some(secret))?;
        }
        crate::credentials::read(KEY_NAME)
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::review_key(secret)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = secret;
        Err("Protected activity storage requires Windows or macOS".into())
    }
}
fn key() -> Result<[u8; 32], String> {
    #[cfg(test)]
    {
        let _ = KEY_NAME;
        Ok([41; 32])
    }
    #[cfg(not(test))]
    {
        static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
        if let Some(key) = KEY.get() {
            return Ok(*key);
        }
        static ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ACCESS
            .lock()
            .map_err(|_| "Protected activity storage unavailable")?;
        if let Some(key) = KEY.get() {
            return Ok(*key);
        }
        let encoded = if let Some(encoded) = vault(None)? {
            encoded
        } else {
            let mut generated = [0; 32];
            SystemRandom::new()
                .fill(&mut generated)
                .map_err(|_| "Could not protect review evidence")?;
            vault(Some(&STANDARD.encode(generated)))?.ok_or("Could not save the review key")?
        };
        let bytes: [u8; 32] = STANDARD
            .decode(encoded)
            .map_err(|_| "Invalid activity storage key")?
            .try_into()
            .map_err(|_| "Invalid activity storage key")?;
        let _ = KEY.set(bytes);
        Ok(bytes)
    }
}
fn seal(bytes: &[u8], aad: &str) -> Result<Vec<u8>, String> {
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key()?)
        .map_err(|_| "Could not protect review evidence")?;
    let mut nonce = [0; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| "Could not protect review evidence")?;
    let mut encrypted = bytes.to_vec();
    aead::LessSafeKey::new(key)
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad.as_bytes()),
            &mut encrypted,
        )
        .map_err(|_| "Could not protect review evidence")?;
    let mut out = nonce.to_vec();
    out.extend(encrypted);
    Ok(out)
}
fn open(mut bytes: Vec<u8>, aad: &str) -> Result<Vec<u8>, String> {
    if bytes.len() < 28 {
        return Err("Review evidence is unavailable".into());
    }
    let nonce: [u8; 12] = bytes[..12]
        .try_into()
        .map_err(|_| "Review evidence is unavailable")?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key()?)
        .map_err(|_| "Review evidence is unavailable")?;
    let decrypted = aead::LessSafeKey::new(key)
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad.as_bytes()),
            &mut bytes[12..],
        )
        .map_err(|_| "Review evidence is unavailable")?;
    Ok(decrypted.to_vec())
}
impl Storage {
    /// How much of the encrypted evidence store is in use, and its ceiling.
    pub(crate) fn review_vault_usage(&self) -> Result<(i64, i64), String> {
        let bytes: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(SUM(length(encrypted)),0) FROM review_evidence",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        Ok((bytes, MAX_REVIEW_BYTES))
    }
    pub(super) fn unreviewed_image(
        &self,
        period: i64,
    ) -> Result<Option<(i64, String, String)>, String> {
        // Describing images is the costly part of a review, one call each. A period gets a
        // bounded number of descriptions; the rest of its blocks are reviewed on their
        // retained text and the descriptions already made.
        let described: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM review_evidence WHERE period_id=?1 AND kind='image' AND analyzed=1",
                [period],
                |r| r.get(0),
            )
            .map_err(db)?;
        if described >= MAX_DESCRIBED_IMAGES {
            return Ok(None);
        }
        let row:Option<(i64,String)>=self.connection.query_row("SELECT e.id,e.context_key FROM review_evidence e JOIN review_contexts c ON c.period_id=e.period_id AND c.context_key=e.context_key WHERE e.period_id=?1 AND e.kind='image' AND e.analyzed=0 AND c.final_reviewed=0 AND c.description IN ('','Activity could not be identified confidently') AND EXISTS(SELECT 1 FROM review_intervals i WHERE i.period_id=e.period_id AND i.context_key=e.context_key AND i.status='pending' AND i.manual=0) ORDER BY e.id DESC LIMIT 1",[period],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db)?;
        row.map(|(id, context)| {
            let image = self.review_image(id)?;
            Ok((
                id,
                context,
                image
                    .trim_start_matches("data:image/jpeg;base64,")
                    .to_string(),
            ))
        })
        .transpose()
    }
    pub(super) fn save_reviewed_image(
        &self,
        period: i64,
        id: i64,
        context: &str,
        description: &str,
    ) -> Result<(), String> {
        crate::memory::atomic(&self.connection, || {
            self.connection.execute("UPDATE review_contexts SET description=?1 WHERE period_id=?2 AND context_key=?3 AND EXISTS(SELECT 1 FROM review_periods p WHERE p.id=?2 AND p.summary IS NULL)",params![crate::companion::redact_sample(description).chars().take(1000).collect::<String>(),period,context]).map_err(db)?;
            self.connection.execute("UPDATE review_evidence SET analyzed=1 WHERE id=?1 AND period_id=?2 AND context_key=?3",params![id,period,context]).map_err(db)?;
            Ok(())
        })
    }
    pub(crate) fn retain_visual_description(
        &self,
        a: &ActivitySnapshot,
        description: &str,
    ) -> Result<(), String> {
        let at = DateTime::parse_from_rfc3339(&a.timestamp)
            .map_err(|_| "Invalid observation time")?
            .with_timezone(&Utc);
        let period = self.review_period(at)?;
        if let Some(context) = self.review_context_key(period, a)? {
            self.connection.execute("UPDATE review_contexts SET description=?1 WHERE period_id=?2 AND context_key=?3 AND description IN ('','Activity could not be identified confidently') AND EXISTS(SELECT 1 FROM review_periods p WHERE p.id=?2 AND p.summary IS NULL)",params![crate::companion::redact_sample(description).chars().take(1000).collect::<String>(),period,context]).map_err(db)?;
        }
        Ok(())
    }
    pub(crate) fn retain_review_evidence(
        &self,
        a: &ActivitySnapshot,
        kind: &str,
        text: &str,
    ) -> Result<(), String> {
        if !["text", "image"].contains(&kind) || text.is_empty() {
            return Ok(());
        }
        if kind == "image" && text.len() > 400_000 {
            return Err("Review image exceeds the storage limit".into());
        }
        let at = DateTime::parse_from_rfc3339(&a.timestamp)
            .map_err(|_| "Invalid observation time")?
            .with_timezone(&Utc);
        let period = self.review_period(at)?;
        let finalized: bool = self
            .connection
            .query_row(
                "SELECT summary IS NOT NULL FROM review_periods WHERE id=?1",
                [period],
                |r| r.get(0),
            )
            .map_err(db)?;
        if finalized {
            return Ok(());
        }
        let Some(context) = self.review_context_key(period, a)? else {
            return Ok(());
        };
        let fingerprint = crate::companion::engine::fingerprint(text);
        let exists:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM review_evidence WHERE period_id=?1 AND context_key=?2 AND kind=?3 AND fingerprint=?4)",params![period,context,kind,fingerprint],|r|r.get(0)).map_err(db)?;
        if exists {
            return Ok(());
        }
        // One image per context segment is enough for the review; later frames of the
        // same five-minute presence add storage, not evidence.
        if kind == "image" {
            let pictured:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM review_evidence WHERE period_id=?1 AND context_key=?2 AND kind='image')",params![period,context],|r|r.get(0)).map_err(db)?;
            if pictured {
                return Ok(());
            }
        }
        let bytes: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(SUM(length(encrypted)),0) FROM review_evidence",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        if bytes + text.len() as i64 + 28 > MAX_REVIEW_BYTES {
            return Err("Activity evidence storage is full. The daily summary must succeed, or local history must be cleared, before more evidence can be saved.".into());
        }
        let content = if kind == "text" {
            crate::companion::redact_sample(text)
        } else {
            text.to_string()
        };
        let encrypted = seal(content.as_bytes(), &format!("{period}:{context}:{kind}"))?;
        self.connection.execute("INSERT OR IGNORE INTO review_evidence(period_id,context_key,kind,fingerprint,captured_at,encrypted) VALUES(?1,?2,?3,?4,?5,?6)",params![period,context,kind,fingerprint,a.timestamp,encrypted]).map_err(db)?;
        Ok(())
    }
    pub(super) fn review_image(&self, id: i64) -> Result<String, String> {
        let (period,context,kind,encrypted)=self.connection.query_row("SELECT e.period_id,e.context_key,e.kind,e.encrypted FROM review_evidence e JOIN review_periods p ON p.id=e.period_id WHERE e.id=?1 AND e.kind='image' AND p.summary IS NULL",[id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Vec<u8>>(3)?))).map_err(|_|"This screenshot was deleted after the daily summary")?;
        let bytes = open(encrypted, &format!("{period}:{context}:{kind}"))?;
        let image = String::from_utf8(bytes).map_err(|_| "Review screenshot is unavailable")?;
        Ok(format!("data:image/jpeg;base64,{image}"))
    }
    pub(super) fn review_text(&self, period: i64, context: &str) -> Result<String, String> {
        let mut q=self.connection.prepare("SELECT encrypted FROM review_evidence WHERE period_id=?1 AND context_key=?2 AND kind='text' ORDER BY id DESC LIMIT 2").map_err(db)?;
        let encrypted = q
            .query_map(params![period, context], |r| r.get::<_, Vec<u8>>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let aad = format!("{period}:{context}:text");
        let excerpts = encrypted
            .into_iter()
            .map(|bytes| {
                String::from_utf8(open(bytes, &aad)?)
                    .map_err(|_| "Review evidence is unavailable".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(excerpts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_evidence_is_bound_to_its_context_and_detects_tampering() {
        let bytes = seal(b"synthetic image", "1:window:image").unwrap();
        assert!(!bytes.windows(15).any(|w| w == b"synthetic image"));
        assert_eq!(
            open(bytes.clone(), "1:window:image").unwrap(),
            b"synthetic image"
        );
        assert!(open(bytes.clone(), "2:window:image").is_err());
        let mut damaged = bytes;
        damaged[20] ^= 1;
        assert!(open(damaged, "1:window:image").is_err());
    }
}
