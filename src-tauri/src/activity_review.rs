//! Measured activity, reversible goal assignment and durable work-period summaries.
//! The existing collector owns time. This module never estimates elapsed duration.
mod evidence;
mod storage;
mod worker;
pub(crate) use worker::run_due;
#[cfg(test)]
mod tests;

use crate::{commands::AppState, models::ActivitySnapshot, storage::Storage};
use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use rusqlite::params;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewWindow};

pub(crate) struct PlanningContext {
    pub goal_id: Option<i64>,
    pub at: std::time::Instant,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub key: String,
    pub period_id: i64,
    pub day: String,
    pub started_at: String,
    pub ended_at: String,
    pub seconds: u64,
    pub apps: String,
    pub title: String,
    pub description: String,
    pub proposed_title: String,
    pub reason: String,
    pub status: String,
    pub goal_id: Option<i64>,
    pub goal_title: Option<String>,
    pub evidence_ids: Vec<i64>,
    pub finalized: bool,
}
#[derive(Serialize)]
pub struct Overview {
    pub pending_seconds: u64,
    pub pending_summary_count: u64,
    pub blocks: Vec<Block>,
    pub summaries: Vec<Summary>,
}
#[derive(Serialize, Deserialize)]
pub struct Summary {
    pub period_id: i64,
    pub day: String,
    pub saved_at: String,
    pub blocks: Vec<Block>,
    pub results: Vec<String>,
}
pub(crate) struct Period {
    day: String,
    starts_at: String,
    ends_at: String,
}
fn db(error: rusqlite::Error) -> String {
    error.to_string()
}
pub(crate) fn context_key(activity: &ActivitySnapshot) -> String {
    crate::companion::engine::fingerprint(
        &serde_json::json!([
            activity.process_name.to_lowercase(),
            crate::browser::minimize_title(&activity.window_title),
            activity.browser,
            activity.document_key
        ])
        .to_string(),
    )
}
/// Daily review follows the configurable Daily summary time, independently of
/// notification delivery. Each saved boundary survives settings changes.
fn period_at<T: TimeZone>(at: DateTime<T>, minute: u32) -> Result<Period, String> {
    let zone = at.timezone();
    let today = local_boundary(&zone, at.date_naive(), minute)?;
    let day = at.date_naive() + Duration::days(i64::from(today <= at));
    let end = local_boundary(&zone, day, minute)?;
    let start = local_boundary(&zone, day - Duration::days(1), minute)?;
    Ok(Period {
        day: day.to_string(),
        starts_at: start.with_timezone(&Utc).to_rfc3339(),
        ends_at: end.with_timezone(&Utc).to_rfc3339(),
    })
}
fn local_boundary<T: TimeZone>(
    zone: &T,
    day: NaiveDate,
    minute: u32,
) -> Result<DateTime<T>, String> {
    let local = day
        .and_hms_opt(minute / 60, minute % 60, 0)
        .ok_or("Invalid working hours")?;
    // Use the later occurrence during a repeated hour; advance across a DST gap.
    (0..=180)
        .find_map(|offset| {
            zone.from_local_datetime(&(local + Duration::minutes(offset)))
                .latest()
        })
        .ok_or_else(|| "Working hours are unavailable in this time zone".into())
}
impl Storage {
    pub(crate) fn initialize_activity_review(&self) -> Result<(), String> {
        self.connection
            .execute_batch(include_str!("activity_review/schema.sql"))
            .map_err(db)
    }
    fn review_period(&self, at: DateTime<Utc>) -> Result<i64, String> {
        let existing:Option<i64>=self.connection.query_row("SELECT id FROM review_periods WHERE summary IS NULL AND julianday(starts_at)<=julianday(?1) AND julianday(ends_at)>julianday(?1) ORDER BY id LIMIT 1",[at.to_rfc3339()],|r|r.get(0)).optional().map_err(db)?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let minute = self
            .read_setting::<crate::companion::engine::Preferences>("companion_preferences")?
            .unwrap_or_default()
            .summary_minute;
        let period = period_at(at.with_timezone(&Local), minute)?;
        self.connection
            .execute(
                "INSERT OR IGNORE INTO review_periods(day,starts_at,ends_at) VALUES(?1,?2,?3)",
                params![period.day, period.starts_at, period.ends_at],
            )
            .map_err(db)?;
        self.connection
            .query_row(
                "SELECT id FROM review_periods WHERE ends_at=?1",
                [period.ends_at],
                |r| r.get(0),
            )
            .map_err(db)
    }
    /// Locate evidence by its own observation time, not the latest window with
    /// the same generic title. Separate visits and long sessions stay distinct.
    fn review_context_key(
        &self,
        period: i64,
        a: &ActivitySnapshot,
    ) -> Result<Option<String>, String> {
        let prefix = format!("{}:", context_key(a));
        self.connection.query_row("SELECT context_key FROM review_intervals WHERE period_id=?1 AND substr(context_key,1,length(?2))=?2 AND julianday(started_at)<=julianday(?3) AND julianday(ended_at)>=julianday(?3) ORDER BY id DESC LIMIT 1",params![period,prefix,a.timestamp],|r|r.get(0)).optional().map_err(db)
    }
    pub(crate) fn finish_due_reviews(&mut self, now: DateTime<Utc>) -> Result<(), String> {
        let mut query = self.connection.prepare("SELECT id,day FROM review_periods WHERE summary IS NULL AND analysis_done=1 AND julianday(ends_at)<=julianday(?1) ORDER BY ends_at LIMIT 8").map_err(db)?;
        let periods = query
            .query_map([now.to_rfc3339()], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        drop(query);
        for (id, day) in periods {
            let mut blocks = self.review_blocks(Some(id))?;
            for block in &mut blocks {
                block.evidence_ids.clear();
                block.finalized = true;
            }
            let mut q = self.connection.prepare("SELECT title || ' (' || status || ')' FROM work_events WHERE julianday(occurred_at)>=(SELECT julianday(starts_at) FROM review_periods WHERE id=?1) AND julianday(occurred_at)<(SELECT julianday(ends_at) FROM review_periods WHERE id=?1) AND status!='observed' ORDER BY id").map_err(db)?;
            let results = q
                .query_map([id], |r| r.get::<_, String>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            drop(q);
            let summary = Summary {
                period_id: id,
                day,
                saved_at: now.to_rfc3339(),
                blocks,
                results,
            };
            let encoded =
                serde_json::to_string(&summary).map_err(|_| "Could not prepare daily summary")?;
            let tx = self.connection.transaction().map_err(db)?;
            // Summary and cleanup share one durable commit. Failure preserves evidence.
            let saved = tx.execute("UPDATE review_periods SET summary=?1,finalized_at=?2 WHERE id=?3 AND summary IS NULL",params![encoded,now.to_rfc3339(),id]).map_err(db)?;
            if saved == 1 {
                tx.execute("DELETE FROM review_evidence WHERE period_id=?1", [id])
                    .map_err(db)?;
            }
            tx.commit().map_err(db)?;
        }
        Ok(())
    }
}

fn workspace(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Open Today to review activity".into())
    }
}
#[tauri::command(async)]
pub fn get_activity_review(
    window: WebviewWindow,
    state: State<AppState>,
) -> Result<Overview, String> {
    workspace(&window)?;
    let inner = state
        .inner
        .lock()
        .map_err(|_| "Activity review unavailable")?;
    inner.storage.activity_review()
}
#[tauri::command(async)]
pub fn review_activity(
    window: WebviewWindow,
    state: State<AppState>,
    period_id: i64,
    key: String,
    action: String,
    goal_id: Option<i64>,
    title: Option<String>,
) -> Result<(), String> {
    workspace(&window)?;
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Activity review unavailable")?;
    inner
        .storage
        .review_action(period_id, &key, &action, goal_id, title.as_deref())?;
    inner.privacy_revision += 1;
    inner.goal_matching.clear();
    inner.unplanned_work.clear();
    crate::frontend_events::changed(window.app_handle(), crate::frontend_events::ALL, false);
    Ok(())
}
#[tauri::command(async)]
pub fn get_activity_evidence(
    window: WebviewWindow,
    state: State<AppState>,
    id: i64,
) -> Result<String, String> {
    workspace(&window)?;
    let inner = state
        .inner
        .lock()
        .map_err(|_| "Activity review unavailable")?;
    inner.storage.review_image(id)
}
#[tauri::command(async)]
pub fn set_activity_review_context(
    window: WebviewWindow,
    state: State<AppState>,
    active: bool,
    goal_id: Option<i64>,
) -> Result<(), String> {
    workspace(&window)?;
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Activity review unavailable")?;
    if let Some(id) = goal_id {
        inner.storage.validate_planning_goal(id)?;
    }
    inner.review_planning = active.then(|| PlanningContext {
        goal_id,
        at: std::time::Instant::now(),
    });
    Ok(())
}
