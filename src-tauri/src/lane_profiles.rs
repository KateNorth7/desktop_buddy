//! Learned lane profiles: what each goal's attributed work looks like, as the sites and
//! documents it was done in. Taught by the app's own confident attributions and by the
//! user's own assignments, never by keyword guesses. A profile that dominates a site or
//! document assigns the next visit locally: no cloud call, and no dependence on the AI
//! budget. Values are the sanitized hostname or the hashed document identity the
//! collector already keeps; titles and text are never stored here.
use crate::{models::ActivitySnapshot, storage::Storage};
use rusqlite::params;

/// Attributed time on one value before its profile may assign on its own.
pub(crate) const MIN_EVIDENCE_MS: i64 = 300_000;
/// The share of a value's attributed time one goal must hold; split evidence only hints.
pub(crate) const DOMINANCE: f64 = 0.8;
/// Attributions at or above this confidence teach a profile.
pub(crate) const MIN_SOURCE_CONFIDENCE: f64 = 0.8;
/// Confidence of a local assignment made from a dominant profile.
pub(crate) const LEARNED_CONFIDENCE: f64 = 0.9;

pub(crate) struct LaneMatch {
    pub goal_id: i64,
    pub reason: String,
}
fn db(e: rusqlite::Error) -> String {
    e.to_string()
}
/// The one key a sample can teach or match: a browser's site, otherwise a document.
/// Title-only browser windows and applications without a document teach nothing.
fn keys(a: &ActivitySnapshot) -> Vec<(&'static str, String)> {
    if let Some(domain) = a
        .browser
        .as_ref()
        .and_then(|b| b.domain.as_deref())
        .and_then(crate::browser::sanitize_domain)
    {
        return vec![("domain", domain)];
    }
    if a.browser.is_none() {
        if let Some(document) = a.document_key.as_deref().filter(|d| !d.trim().is_empty()) {
            return vec![(
                "document",
                format!("{}|{document}", a.process_name.to_lowercase()),
            )];
        }
    }
    Vec::new()
}
fn noun(kind: &str) -> &'static str {
    if kind == "domain" {
        "site"
    } else {
        "document"
    }
}
impl Storage {
    pub(crate) fn initialize_lane_profiles(&self) -> Result<(), String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS goal_lane_profiles(goal_id INTEGER NOT NULL REFERENCES goals(id) ON DELETE CASCADE,kind TEXT NOT NULL CHECK(kind IN ('domain','document')),value TEXT NOT NULL,milliseconds INTEGER NOT NULL DEFAULT 0,last_seen TEXT NOT NULL,PRIMARY KEY(goal_id,kind,value)); CREATE INDEX IF NOT EXISTS goal_lane_profiles_value ON goal_lane_profiles(kind,value);").map_err(db)
    }
    /// Attributed time teaches the goal's profile for the sample's site or document.
    pub(crate) fn learn_lane_profile(
        &self,
        goal: i64,
        a: &ActivitySnapshot,
        milliseconds: i64,
        confidence: f64,
    ) -> Result<(), String> {
        if milliseconds <= 0 || !confidence.is_finite() || confidence < MIN_SOURCE_CONFIDENCE {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        for (kind, value) in keys(a) {
            self.connection.execute("INSERT INTO goal_lane_profiles(goal_id,kind,value,milliseconds,last_seen) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(goal_id,kind,value) DO UPDATE SET milliseconds=milliseconds+excluded.milliseconds,last_seen=excluded.last_seen",params![goal,kind,value,milliseconds,now]).map_err(db)?;
        }
        Ok(())
    }
    /// A manual move away from a goal withdraws that goal's evidence for the value.
    pub(crate) fn unlearn_lane_profile(
        &self,
        goal: i64,
        a: &ActivitySnapshot,
        milliseconds: i64,
    ) -> Result<(), String> {
        for (kind, value) in keys(a) {
            self.connection.execute("UPDATE goal_lane_profiles SET milliseconds=MAX(0,milliseconds-?4) WHERE goal_id=?1 AND kind=?2 AND value=?3",params![goal,kind,value,milliseconds.max(0)]).map_err(db)?;
        }
        Ok(())
    }
    fn profile_rows(&self, kind: &str, value: &str) -> Result<Vec<(i64, i64)>, String> {
        let mut q = self.connection.prepare("SELECT goal_id,milliseconds FROM goal_lane_profiles WHERE kind=?1 AND value=?2 AND milliseconds>0 ORDER BY milliseconds DESC,goal_id").map_err(db)?;
        let rows = q
            .query_map(params![kind, value], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(rows)
    }
    /// Among the candidate goals, the one whose profile dominates this sample's site or
    /// document. Evidence held by any other goal, candidate or not, counts against dominance.
    pub(crate) fn learned_lane_match(
        &self,
        a: &ActivitySnapshot,
        candidates: &[i64],
    ) -> Result<Option<LaneMatch>, String> {
        for (kind, value) in keys(a) {
            let rows = self.profile_rows(kind, &value)?;
            let total: i64 = rows.iter().map(|r| r.1).sum();
            let Some(best) = rows.iter().find(|r| candidates.contains(&r.0)) else {
                continue;
            };
            if best.1 >= MIN_EVIDENCE_MS && best.1 as f64 >= DOMINANCE * total as f64 {
                return Ok(Some(LaneMatch {
                    goal_id: best.0,
                    reason: format!(
                        "Earlier attributed work on this {} belongs to this goal",
                        noun(kind)
                    ),
                }));
            }
        }
        Ok(None)
    }
    /// Goals with any attributed time on this sample's site or document: evidence for the
    /// matcher to weigh, never an assignment on its own.
    pub(crate) fn learned_lane_hints(&self, a: &ActivitySnapshot) -> Result<Vec<i64>, String> {
        let mut goals = Vec::new();
        for (kind, value) in keys(a) {
            for (goal, _) in self.profile_rows(kind, &value)? {
                if !goals.contains(&goal) {
                    goals.push(goal);
                }
            }
        }
        Ok(goals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (Storage, i64, i64) {
        let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
        let a = s.set_goal("Apply to jobs").unwrap().id;
        let b = s.set_goal("Create CV versions").unwrap().id;
        (s, a, b)
    }
    fn site(domain: &str) -> ActivitySnapshot {
        ActivitySnapshot {
            process_name: "safari".into(),
            window_title: "Register".into(),
            browser: crate::browser::context("safari", "Register", Some(domain.into())),
            ..Default::default()
        }
    }
    fn document(app: &str, key: &str) -> ActivitySnapshot {
        ActivitySnapshot {
            process_name: app.into(),
            window_title: app.into(),
            document_key: Some(key.into()),
            ..Default::default()
        }
    }
    #[test]
    fn a_dominant_profile_assigns_and_split_or_thin_evidence_only_hints() {
        let (s, a, b) = store();
        let loreal = site("careers.loreal.com");
        // Below five attributed minutes nothing assigns, though the hint exists.
        s.learn_lane_profile(a, &loreal, 240_000, 0.86).unwrap();
        assert!(s.learned_lane_match(&loreal, &[a, b]).unwrap().is_none());
        assert_eq!(s.learned_lane_hints(&loreal).unwrap(), vec![a]);
        s.learn_lane_profile(a, &loreal, 120_000, 0.9).unwrap();
        let found = s.learned_lane_match(&loreal, &[a, b]).unwrap().unwrap();
        assert_eq!(found.goal_id, a);
        assert!(found.reason.contains("site"));
        // Low-confidence or empty attributions teach nothing.
        s.learn_lane_profile(b, &loreal, 600_000, 0.7).unwrap();
        s.learn_lane_profile(b, &loreal, 0, 0.95).unwrap();
        assert_eq!(
            s.learned_lane_match(&loreal, &[a, b])
                .unwrap()
                .unwrap()
                .goal_id,
            a
        );
        // Split evidence: a holds 360 s, b 120 s = 75%, under the 80% bar.
        s.learn_lane_profile(b, &loreal, 120_000, 0.9).unwrap();
        assert!(s.learned_lane_match(&loreal, &[a, b]).unwrap().is_none());
        assert_eq!(s.learned_lane_hints(&loreal).unwrap(), vec![a, b]);
        // A manual move away withdraws evidence.
        s.unlearn_lane_profile(b, &loreal, 120_000).unwrap();
        assert_eq!(
            s.learned_lane_match(&loreal, &[a, b])
                .unwrap()
                .unwrap()
                .goal_id,
            a
        );
        // Only the candidates may be chosen, but every goal's evidence counts against dominance.
        assert!(s.learned_lane_match(&loreal, &[b]).unwrap().is_none());
        s.learn_lane_profile(b, &loreal, 300_000, 0.9).unwrap();
        assert!(s.learned_lane_match(&loreal, &[a]).unwrap().is_none());
    }
    #[test]
    fn documents_teach_per_application_and_title_only_windows_teach_nothing() {
        let (mut s, a, b) = store();
        let chat = document("claude", "abc123");
        s.learn_lane_profile(b, &chat, 400_000, 0.9).unwrap();
        assert_eq!(
            s.learned_lane_match(&chat, &[a, b])
                .unwrap()
                .unwrap()
                .goal_id,
            b
        );
        assert!(s
            .learned_lane_match(&chat, &[a, b])
            .unwrap()
            .unwrap()
            .reason
            .contains("document"));
        // Another application's document with the same hash is a different value.
        assert!(s
            .learned_lane_match(&document("notes", "abc123"), &[a, b])
            .unwrap()
            .is_none());
        // A browser window without an address, or an app without a document, has no key.
        let mut bare = site("careers.loreal.com");
        bare.browser = crate::browser::context("safari", "Register", None);
        s.learn_lane_profile(a, &bare, 900_000, 0.95).unwrap();
        assert!(s.learned_lane_match(&bare, &[a, b]).unwrap().is_none());
        let plain = ActivitySnapshot {
            process_name: "telegram".into(),
            window_title: "Telegram".into(),
            ..Default::default()
        };
        s.learn_lane_profile(a, &plain, 900_000, 0.95).unwrap();
        assert!(s.learned_lane_hints(&plain).unwrap().is_empty());
        let stored: i64 = s
            .connection
            .query_row("SELECT COUNT(*) FROM goal_lane_profiles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, 1, "only the document row exists");
        // Deleting the goal removes its profile; clearing local history removes all.
        s.connection
            .execute("DELETE FROM goals WHERE id=?1", [b])
            .unwrap();
        assert!(s.learned_lane_hints(&chat).unwrap().is_empty());
        s.learn_lane_profile(a, &site("example.com"), 600_000, 0.9)
            .unwrap();
        s.purge_history(Some("2000-01-01T00:00:00Z")).unwrap();
        assert_eq!(s.learned_lane_hints(&site("example.com")).unwrap(), vec![a]);
        s.purge_history(None).unwrap();
        assert!(s
            .learned_lane_hints(&site("example.com"))
            .unwrap()
            .is_empty());
    }
}
