//! Durable journal of goal attribution and outcome-check attempts. The session-only engine
//! journal disappears on restart, so a day where no time reached a goal could not be
//! explained afterwards. Rows hold only fixed outcome codes, a goal ID and a confidence:
//! never titles, text, prompts or model reasoning. Repeats of the same outcome are folded
//! into one row, so a lane that stays busy for an hour cannot flood the table.
use crate::storage::Storage;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

/// Goal attribution: which goal the current activity serves.
pub(crate) const MATCHING: &str = "goal_matching";
/// Outcome checks: whether the activity shows a goal or step is done.
pub(crate) const OUTCOME: &str = "outcome_check";
/// The same outcome within this many seconds extends the previous row.
const FOLD_SECONDS: i64 = 300;
/// Rows older than this are removed on write; retention and Clear local history also apply.
const KEEP_DAYS: i64 = 14;

fn db(e: rusqlite::Error) -> String {
    e.to_string()
}
fn safe(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
/// The most frequent reason time stayed unassigned on a day, for My progress.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TrackingDay {
    pub recorded_seconds: i64,
    pub attributed_seconds: i64,
    pub waiting_seconds: i64,
    pub issue: Option<String>,
}
/// Outcomes that explain why recorded time did not reach a goal.
const ISSUES: &[&str] = &[
    "skipped_lane_busy",
    "skipped_hourly_budget",
    "provider_rate_limit",
    "provider_timeout_or_network",
    "provider_error",
    "answered_no_goal",
    "answered_below_threshold",
];
impl Storage {
    pub(crate) fn initialize_tracking_attempts(&self) -> Result<(), String> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS tracking_attempts(id INTEGER PRIMARY KEY,stage TEXT NOT NULL,outcome TEXT NOT NULL,goal_id INTEGER REFERENCES goals(id) ON DELETE SET NULL,confidence REAL,first_at TEXT NOT NULL,last_at TEXT NOT NULL,repeats INTEGER NOT NULL DEFAULT 1);
                 CREATE INDEX IF NOT EXISTS tracking_attempts_last ON tracking_attempts(last_at);",
            )
            .map_err(db)
    }
    /// Records one attempt. Unknown codes are dropped rather than stored.
    pub(crate) fn note_attempt(
        &self,
        stage: &str,
        outcome: &str,
        goal_id: Option<i64>,
        confidence: Option<f64>,
    ) -> Result<(), String> {
        if !safe(stage) || !safe(outcome) {
            return Ok(());
        }
        let now = chrono::Utc::now();
        let confidence = confidence.filter(|c| c.is_finite());
        let previous: Option<(i64, String, Option<i64>, String)> = self
            .connection
            .query_row(
                "SELECT id,outcome,goal_id,last_at FROM tracking_attempts WHERE stage=?1 ORDER BY id DESC LIMIT 1",
                [stage],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(db)?;
        let fold = previous.filter(|(_, code, goal, at)| {
            code == outcome
                && *goal == goal_id
                && chrono::DateTime::parse_from_rfc3339(at).is_ok_and(|at| {
                    (now - at.with_timezone(&chrono::Utc)).num_seconds() <= FOLD_SECONDS
                })
        });
        if let Some((id, ..)) = fold {
            self.connection
                .execute(
                    "UPDATE tracking_attempts SET last_at=?2,repeats=repeats+1,confidence=COALESCE(?3,confidence) WHERE id=?1",
                    params![id, now.to_rfc3339(), confidence],
                )
                .map_err(db)?;
            return Ok(());
        }
        self.connection
            .execute(
                "INSERT INTO tracking_attempts(stage,outcome,goal_id,confidence,first_at,last_at) VALUES(?1,?2,?3,?4,?5,?5)",
                params![stage, outcome, goal_id, confidence, now.to_rfc3339()],
            )
            .map_err(db)?;
        self.connection
            .execute(
                "DELETE FROM tracking_attempts WHERE julianday(last_at)<julianday(?1)",
                [(now - chrono::Duration::days(KEEP_DAYS)).to_rfc3339()],
            )
            .map_err(db)?;
        Ok(())
    }
    /// Recorded, attributed and waiting time for one local day, plus the most frequent
    /// reason attribution did not happen that day.
    pub(crate) fn tracking_day(&self, day: &str) -> Result<TrackingDay, String> {
        // Progress is also read while older databases migrate, before the review tables exist.
        let ready: bool = self
            .connection
            .query_row(
                "SELECT COUNT(*)=2 FROM sqlite_master WHERE type='table' AND name IN ('review_intervals','tracking_attempts')",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !ready {
            return Ok(TrackingDay::default());
        }
        let (recorded, attributed, waiting): (i64, i64, i64) = self
            .connection
            .query_row(
                "SELECT COALESCE(SUM(milliseconds),0),COALESCE(SUM(CASE WHEN status='assigned' THEN milliseconds ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='pending' THEN milliseconds ELSE 0 END),0) FROM review_intervals WHERE date(started_at,'localtime')=?1",
                [day],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(db)?;
        let issue = if waiting > 0 {
            let codes = ISSUES
                .iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join(",");
            self.connection
                .query_row(
                    &format!("SELECT outcome FROM tracking_attempts WHERE stage=?1 AND outcome IN ({codes}) AND date(last_at,'localtime')=?2 GROUP BY outcome ORDER BY SUM(repeats) DESC,outcome LIMIT 1"),
                    params![MATCHING, day],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?
        } else {
            None
        };
        Ok(TrackingDay {
            recorded_seconds: recorded / 1000,
            attributed_seconds: attributed / 1000,
            waiting_seconds: waiting / 1000,
            issue,
        })
    }
}
/// Maps a provider error to the code stored for it.
pub(crate) fn provider_outcome(error: &str) -> &'static str {
    match crate::engine_observability::error_category(error) {
        "provider_rate_limit" => "provider_rate_limit",
        "provider_timeout_or_network" => "provider_timeout_or_network",
        _ => "provider_error",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn storage() -> Storage {
        Storage::open(std::path::Path::new(":memory:")).unwrap()
    }
    fn rows(s: &Storage) -> Vec<(String, i64)> {
        let mut q = s
            .connection
            .prepare("SELECT outcome,repeats FROM tracking_attempts ORDER BY id")
            .unwrap();
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
    #[test]
    fn repeats_fold_and_unsafe_codes_are_dropped() {
        let s = storage();
        s.note_attempt(MATCHING, "skipped_lane_busy", None, None)
            .unwrap();
        s.note_attempt(MATCHING, "skipped_lane_busy", None, None)
            .unwrap();
        s.note_attempt(MATCHING, "answered_no_goal", None, Some(0.2))
            .unwrap();
        s.note_attempt(MATCHING, "Window title: secret", None, None)
            .unwrap();
        s.note_attempt(OUTCOME, "skipped_lane_busy", None, None)
            .unwrap();
        assert_eq!(
            rows(&s),
            vec![
                ("skipped_lane_busy".into(), 2),
                ("answered_no_goal".into(), 1),
                ("skipped_lane_busy".into(), 1)
            ]
        );
    }
    #[test]
    fn a_day_reports_recorded_attributed_waiting_and_the_main_issue() {
        let s = storage();
        let day = chrono::Local::now().date_naive().to_string();
        assert_eq!(s.tracking_day(&day).unwrap(), TrackingDay::default());
        for _ in 0..3 {
            s.note_attempt(MATCHING, "skipped_lane_busy", None, None)
                .unwrap();
        }
        s.note_attempt(MATCHING, "answered_no_goal", None, None)
            .unwrap();
        s.note_attempt(MATCHING, "assigned", None, Some(0.9))
            .unwrap();
        // Nothing waits yet, so no issue is reported.
        assert_eq!(s.tracking_day(&day).unwrap().issue, None);
        let interval = |offset: i64| crate::history::UsageInterval {
            start: chrono::Utc::now() - chrono::Duration::seconds(offset + 10),
            end: chrono::Utc::now() - chrono::Duration::seconds(offset),
            activity: crate::models::ActivitySnapshot {
                process_name: "Editor".into(),
                window_title: "Notes".into(),
                ..Default::default()
            },
            state: crate::tracking::ActivityState::Focused,
        };
        let unmatched = crate::relevance::ActivityMatch {
            goal_id: None,
            confidence: 0.,
            reason: "No verified relevance".into(),
        };
        s.record_review_interval(&interval(30), &unmatched).unwrap();
        s.record_review_interval(&interval(10), &unmatched).unwrap();
        let today = s.tracking_day(&day).unwrap();
        assert_eq!(today.recorded_seconds, 20);
        assert_eq!(today.attributed_seconds, 0);
        assert_eq!(today.waiting_seconds, 20);
        // Folded repeats count: three busy-lane skips outweigh one empty answer, and a
        // successful assignment is never reported as an issue.
        assert_eq!(today.issue.as_deref(), Some("skipped_lane_busy"));
    }
}
