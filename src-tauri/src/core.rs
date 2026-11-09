use crate::{
    commands::AppState,
    goals::{self, GoalPlan, Step},
    storage::Storage,
};
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CorePreferences {
    pub working_days: Vec<u8>,
    pub movement_reminders: bool,
    pub movement_interval_minutes: u32,
    pub vision_model: String,
    pub ai_connection_mode: crate::shared_ai::ConnectionMode,
}
impl Default for CorePreferences {
    fn default() -> Self {
        Self {
            working_days: vec![1, 2, 3, 4, 5],
            movement_reminders: true,
            movement_interval_minutes: 45,
            vision_model: String::new(),
            ai_connection_mode: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timer {
    pub goal_id: i64,
    pub last_tick: DateTime<Utc>,
}
#[derive(Serialize)]
pub struct Area {
    pub id: i64,
    pub title: String,
}
#[derive(Serialize)]
pub struct CoreGoal {
    pub id: i64,
    pub title: String,
    pub status: String,
    pub area_id: i64,
    pub plan: GoalPlan,
    pub focused_seconds: u64,
    pub tracked_seconds: u64,
    pub relevant_seconds: u64,
    pub analysis: crate::automatic_goals::AnalysisView,
    pub due_at: Option<String>,
    pub priority: Option<String>,
    pub description: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    /// Expected minutes for the whole goal (user-set, also used by the chat's overrun check).
    #[serde(default)]
    pub estimated_minutes: Option<i64>,
    #[serde(default)]
    pub work_context: Option<crate::memory::WorkContext>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct GoalDetails {
    pub due_at: Option<String>,
    pub priority: Option<String>,
    pub description: String,
}
impl GoalDetails {
    fn validate(mut self) -> Result<Self, String> {
        if let Some(value) = &self.due_at {
            let parsed =
                DateTime::parse_from_rfc3339(value).map_err(|_| "Choose a valid deadline")?;
            self.due_at = Some(parsed.with_timezone(&Utc).to_rfc3339());
        }
        if self
            .priority
            .as_deref()
            .is_some_and(|p| !["low", "medium", "high", "urgent"].contains(&p))
        {
            return Err("Choose a valid priority".into());
        }
        self.description = self.description.trim().into();
        if self.description.chars().count() > 4000 {
            return Err("Use up to 4,000 characters for a description".into());
        }
        Ok(self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalDraft {
    pub title: String,
    pub area: String,
    pub steps: Vec<String>,
}
#[derive(Serialize)]
pub struct StepTime {
    /// Empty when the time was not tied to a step.
    pub step_id: String,
    pub seconds: u64,
}
#[derive(Serialize)]
pub struct GoalTime {
    pub goal_id: i64,
    pub title: String,
    pub area: String,
    pub seconds: u64,
    pub tracked_seconds: u64,
    pub relevant_seconds: u64,
    pub steps: Vec<StepTime>,
}
#[derive(Serialize)]
pub struct DayProgress {
    pub day: String,
    pub completed: Vec<String>,
    pub completed_goals: Vec<String>,
    pub goals: Vec<GoalTime>,
}
#[derive(Serialize)]
pub struct HourTime {
    pub hour: u8,
    pub goal_id: i64,
    pub seconds: u64,
}
#[derive(Serialize)]
pub struct CoreSnapshot {
    pub onboarding_goal_id: Option<i64>,
    pub date: String,
    pub selected_date: String,
    pub date_goals: Vec<i64>,
    pub carryover: Vec<i64>,
    pub day_mode: String,
    pub today: Vec<i64>,
    pub areas: Vec<Area>,
    pub goals: Vec<CoreGoal>,
    pub summary: DayProgress,
    pub week: Vec<DayProgress>,
    /// Relevant focused time of the selected day, split by local hour.
    pub hourly: Vec<HourTime>,
    pub timer: Option<Timer>,
    pub focused_goal_id: Option<i64>,
    pub preferences: CorePreferences,
}
fn db(e: rusqlite::Error) -> String {
    e.to_string()
}
pub(crate) fn record_step_changes(
    tx: &rusqlite::Connection,
    old: &GoalPlan,
    plan: &GoalPlan,
) -> Result<(), String> {
    let now = Utc::now();
    for step in &plan.steps {
        let was_done = old
            .steps
            .iter()
            .find(|previous| previous.id == step.id)
            .is_some_and(|previous| previous.done);
        if was_done != step.done {
            tx.execute("INSERT INTO core_events(goal_id,step_id,title,kind,day,created_at,completion_source) VALUES(?1,?2,?3,?4,?5,?6,'manual')", params![plan.goal_id,step.id,step.text,if step.done {"completed"} else {"reopened"},now.with_timezone(&Local).date_naive().to_string(),now.to_rfc3339()]).map_err(db)?;
        }
    }
    Ok(())
}
pub(crate) fn record_legacy_transition(
    tx: &rusqlite::Connection,
    id: i64,
    action: &str,
) -> Result<(), String> {
    let state = match action {
        "resume" => "open",
        "complete" => "completed",
        _ => "deferred",
    };
    tx.execute(
        "UPDATE core_goals SET state=?1 WHERE goal_id=?2",
        params![state, id],
    )
    .map_err(db)?;
    if action == "complete" {
        let title: String = tx
            .query_row("SELECT text FROM goals WHERE id=?1", [id], |row| row.get(0))
            .map_err(db)?;
        let now = Utc::now();
        tx.execute("INSERT INTO core_events(goal_id,step_id,title,kind,day,created_at,completion_source) VALUES(?1,'goal',?2,'completed',?3,?4,'manual')", params![id,title,now.with_timezone(&Local).date_naive().to_string(),now.to_rfc3339()]).map_err(db)?;
    }
    if action != "resume" {
        let timer: Option<String> = tx
            .query_row(
                "SELECT value FROM preferences WHERE name='core_timer'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db)?;
        if timer
            .and_then(|value| serde_json::from_str::<Option<Timer>>(&value).ok())
            .flatten()
            .is_some_and(|timer| timer.goal_id == id)
        {
            tx.execute(
                "UPDATE preferences SET value='null' WHERE name='core_timer'",
                [],
            )
            .map_err(db)?;
        }
    }
    Ok(())
}
fn day(value: &str) -> Result<NaiveDate, String> {
    if value.len() != 10 {
        return Err("Choose a valid date".into());
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| "Choose a valid date")?;
    if date.to_string() != value {
        return Err("Choose a valid date".into());
    }
    Ok(date)
}
/// Number of days a period covers when it ends on `anchor`: a week is seven days, a month runs
/// from the 1st and a year from 1 January, so a finished month or year is shown whole.
fn period_days(value: Option<&str>, anchor: NaiveDate) -> Result<i64, String> {
    use chrono::Datelike;
    match value.unwrap_or("week") {
        "day" => Ok(1),
        "week" => Ok(7),
        "month" => Ok(anchor.day() as i64),
        "year" => Ok(anchor.ordinal() as i64),
        _ => Err("Choose a valid progress period".into()),
    }
}
fn area_title(value: &str) -> Result<String, String> {
    let title = value.trim();
    if title.chars().count() > 80 || title.chars().any(char::is_control) {
        return Err("Use up to 80 characters for an area".into());
    }
    Ok(if title.is_empty() {
        "General".into()
    } else {
        title.into()
    })
}
pub fn validate_drafts(mut drafts: Vec<GoalDraft>) -> Result<Vec<GoalDraft>, String> {
    if drafts.is_empty() || drafts.len() > 20 {
        return Err("Add between 1 and 20 goals at a time".into());
    }
    for draft in &mut drafts {
        draft.title = goals::text(&draft.title, true)?;
        draft.area = area_title(&draft.area)?;
        if draft.steps.len() > 20 {
            return Err("Use at most 20 steps per goal".into());
        }
        for step in &mut draft.steps {
            *step = goals::text(step, true)?;
        }
    }
    Ok(drafts)
}
impl Storage {
    pub fn core_reorder(&self, ids: Vec<i64>) -> Result<(), String> {
        let current = self.core_snapshot(None)?;
        let expected: std::collections::HashSet<_> =
            current.goals.iter().map(|goal| goal.id).collect();
        let received: std::collections::HashSet<_> = ids.iter().copied().collect();
        if ids.len() != received.len() || expected != received {
            return Err("Goals changed. Refresh before reordering.".into());
        }
        self.write_setting("core_goal_order", &ids)
    }
    pub(crate) fn initialize_core(&self) -> Result<(), String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS core_areas(id INTEGER PRIMARY KEY,title TEXT NOT NULL COLLATE NOCASE UNIQUE);
            CREATE TABLE IF NOT EXISTS core_goals(goal_id INTEGER PRIMARY KEY REFERENCES goals(id) ON DELETE CASCADE,area_id INTEGER NOT NULL REFERENCES core_areas(id),state TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS core_days(day TEXT PRIMARY KEY,mode TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS core_day_items(day TEXT NOT NULL,goal_id INTEGER NOT NULL REFERENCES goals(id) ON DELETE CASCADE,PRIMARY KEY(day,goal_id));
            CREATE TABLE IF NOT EXISTS core_events(id INTEGER PRIMARY KEY,goal_id INTEGER NOT NULL REFERENCES goals(id) ON DELETE CASCADE,step_id TEXT NOT NULL,title TEXT NOT NULL,kind TEXT NOT NULL,day TEXT NOT NULL,created_at TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS core_events_day ON core_events(day);
            CREATE TRIGGER IF NOT EXISTS mascot_completion_signal AFTER INSERT ON core_events
            WHEN NEW.kind='completed' AND COALESCE((SELECT kind FROM core_events WHERE goal_id=NEW.goal_id AND step_id=NEW.step_id AND id<>NEW.id ORDER BY id DESC LIMIT 1),'')<>'completed'
            BEGIN
                INSERT INTO preferences(name,value) VALUES('mascot_completion_revision','1')
                ON CONFLICT(name) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT);
            END;
            CREATE TABLE IF NOT EXISTS core_time(day TEXT NOT NULL,goal_id INTEGER NOT NULL REFERENCES goals(id) ON DELETE CASCADE,seconds INTEGER NOT NULL,PRIMARY KEY(day,goal_id));
            CREATE TABLE IF NOT EXISTS core_import_batches(batch_id TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS deleted_goal_backups(id INTEGER PRIMARY KEY AUTOINCREMENT,title TEXT NOT NULL,deleted_at INTEGER NOT NULL,payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS core_goal_details(goal_id INTEGER PRIMARY KEY REFERENCES goals(id) ON DELETE CASCADE,due_at TEXT,priority TEXT,description TEXT NOT NULL DEFAULT '');
            CREATE TABLE IF NOT EXISTS core_goal_reopen_overrides(goal_id INTEGER PRIMARY KEY REFERENCES goals(id) ON DELETE CASCADE);
            CREATE TABLE IF NOT EXISTS core_carryover_dismissals(day TEXT NOT NULL,goal_id INTEGER NOT NULL REFERENCES goals(id) ON DELETE CASCADE,PRIMARY KEY(day,goal_id));").map_err(db)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS core_quick_batches(batch_id TEXT PRIMARY KEY,goal_id INTEGER REFERENCES goals(id) ON DELETE SET NULL);").map_err(db)?;
        // A manual timer resumes only after an explicit user action on a new app run.
        self.write_setting("core_timer", &Option::<Timer>::None)?;
        self.initialize_goal_automation()?;
        self.adopt_core_goals()
    }
    pub(crate) fn adopt_core_goals(&self) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT OR IGNORE INTO core_areas(title) VALUES('General')",
                [],
            )
            .map_err(db)?;
        self.connection.execute("INSERT OR IGNORE INTO core_goals(goal_id,area_id,state) SELECT id,(SELECT id FROM core_areas WHERE title='General'),CASE status WHEN 'active' THEN 'open' WHEN 'completed' THEN 'completed' ELSE 'deferred' END FROM goals", []).map_err(db)?;
        Ok(())
    }
    pub fn core_add_goals(&mut self, drafts: Vec<GoalDraft>, batch: &str) -> Result<(), String> {
        let drafts = validate_drafts(drafts)?;
        if batch.is_empty()
            || batch.len() > 80
            || !batch
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("Invalid import identifier".into());
        }
        let count: u32 = self
            .connection
            .query_row("SELECT COUNT(*) FROM goals", [], |r| r.get(0))
            .map_err(db)?;
        let tx = self.connection.savepoint().map_err(db)?;
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO core_import_batches(batch_id) VALUES(?1)",
                [batch],
            )
            .map_err(db)?;
        if inserted == 0 {
            return Ok(());
        }
        if count + drafts.len() as u32 > 500 {
            return Err("Keep up to 500 goals; remove an older goal before adding more".into());
        }
        let now = Utc::now().to_rfc3339();
        for draft in drafts {
            tx.execute(
                "INSERT OR IGNORE INTO core_areas(title) VALUES(?1)",
                [&draft.area],
            )
            .map_err(db)?;
            let area: i64 = tx
                .query_row(
                    "SELECT id FROM core_areas WHERE title=?1 COLLATE NOCASE",
                    [&draft.area],
                    |r| r.get(0),
                )
                .map_err(db)?;
            // Keep parallel goals separate from the legacy singleton focus selection.
            tx.execute(
                "INSERT INTO goals(text,status,created_at) VALUES(?1,'deferred',?2)",
                params![draft.title, now],
            )
            .map_err(db)?;
            let id = tx.last_insert_rowid();
            tx.execute("INSERT INTO core_goal_analyses(goal_id) VALUES(?1)", [id])
                .map_err(db)?;
            tx.execute(
                "INSERT INTO core_goals(goal_id,area_id,state) VALUES(?1,?2,'open')",
                params![id, area],
            )
            .map_err(db)?;
            let plan = GoalPlan {
                goal_id: id,
                revision: 1,
                done_when: String::new(),
                current_step: None,
                steps: draft
                    .steps
                    .into_iter()
                    .enumerate()
                    .map(|(i, text)| Step {
                        id: format!("step-{i}"),
                        text,
                        done: false,
                    })
                    .collect(),
            };
            tx.execute(
                "INSERT INTO goal_plans(goal_id,revision,payload) VALUES(?1,1,?2)",
                params![
                    id,
                    serde_json::to_string(&plan).map_err(|_| "Could not save steps")?
                ],
            )
            .map_err(db)?;
        }
        crate::onboarding_goal::goal_created(&tx)?;
        tx.commit().map_err(db)
    }
    #[cfg(test)]
    pub fn core_save_goal(
        &mut self,
        title: &str,
        area: &str,
        plan: GoalPlan,
    ) -> Result<(), String> {
        self.core_save_goal_details(title, area, plan, None)
    }
    pub fn core_save_goal_details(
        &mut self,
        title: &str,
        area: &str,
        plan: GoalPlan,
        details: Option<GoalDetails>,
    ) -> Result<(), String> {
        self.core_save_goal_details_with(title, area, plan, details, false)
    }
    pub(crate) fn core_restore_goal_details(
        &mut self,
        title: &str,
        area: &str,
        plan: GoalPlan,
    ) -> Result<(), String> {
        self.core_save_goal_details_with(title, area, plan, None, true)
    }
    /// Update an observed open goal without changing focus or completing its wider outcome.
    pub(crate) fn core_save_observed_plan(&mut self, plan: GoalPlan) -> Result<(), String> {
        let (title, area): (String, String) = self.connection.query_row(
            "SELECT g.text,a.title FROM goals g JOIN core_goals c ON c.goal_id=g.id JOIN core_areas a ON a.id=c.area_id WHERE g.id=?1 AND c.state='open'",
            [plan.goal_id], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|_| "The observed goal is no longer open")?;
        self.core_restore_goal_details(&title, &area, plan)
    }
    fn core_save_goal_details_with(
        &mut self,
        title: &str,
        area: &str,
        mut plan: GoalPlan,
        details: Option<GoalDetails>,
        restoring: bool,
    ) -> Result<(), String> {
        let details = details.map(GoalDetails::validate).transpose()?;
        let title = goals::text(title, true)?;
        let area = area_title(area)?;
        plan = goals::validate(plan)?;
        let old = self.goal_plan(plan.goal_id)?;
        if old.revision != plan.revision {
            return Err("This goal changed. Reload before saving.".into());
        }
        let tx = self.connection.savepoint().map_err(db)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM core_goals WHERE goal_id=?1)",
                [plan.goal_id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !exists {
            return Err("Goal no longer exists".into());
        }
        tx.execute(
            "INSERT OR IGNORE INTO core_areas(title) VALUES(?1)",
            [&area],
        )
        .map_err(db)?;
        tx.execute("UPDATE core_goals SET area_id=(SELECT id FROM core_areas WHERE title=?1 COLLATE NOCASE) WHERE goal_id=?2", params![area,plan.goal_id]).map_err(db)?;
        tx.execute(
            "UPDATE goals SET text=?1 WHERE id=?2",
            params![title, plan.goal_id],
        )
        .map_err(db)?;
        if let Some(details) = details {
            tx.execute("INSERT INTO core_goal_details(goal_id,due_at,priority,description) VALUES(?1,?2,?3,?4) ON CONFLICT(goal_id) DO UPDATE SET due_at=excluded.due_at,priority=excluded.priority,description=excluded.description", params![plan.goal_id,details.due_at,details.priority,details.description]).map_err(db)?;
        }
        record_step_changes(&tx, &old, &plan)?;
        let old_progress: std::collections::BTreeMap<_, _> =
            old.steps.iter().map(|s| (&s.id, s.done)).collect();
        let new_progress: std::collections::BTreeMap<_, _> =
            plan.steps.iter().map(|s| (&s.id, s.done)).collect();
        if !restoring && old_progress != new_progress {
            tx.execute(
                "DELETE FROM core_goal_reopen_overrides WHERE goal_id=?1",
                [plan.goal_id],
            )
            .map_err(db)?;
        }
        plan.revision = plan
            .revision
            .checked_add(1)
            .ok_or("Goal revision limit reached")?;
        tx.execute("INSERT INTO goal_plans(goal_id,revision,payload) VALUES(?1,?2,?3) ON CONFLICT(goal_id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload", params![plan.goal_id,plan.revision,serde_json::to_string(&plan).map_err(|_| "Could not save steps")?]).map_err(db)?;
        tx.commit().map_err(db)?;
        // Unticking a step of a completed goal reopens it.
        if !restoring && plan.steps.iter().any(|step| !step.done) {
            let state: Option<String> = self
                .connection
                .query_row(
                    "SELECT state FROM core_goals WHERE goal_id=?1",
                    [plan.goal_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?;
            if state.as_deref() == Some("completed") {
                self.core_transition_with(plan.goal_id, "resume", false)?;
            }
        }
        // When every step is done the goal is done too.
        let keep_open: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM core_goal_reopen_overrides WHERE goal_id=?1)",
                [plan.goal_id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !restoring
            && plan.done_when.trim().is_empty()
            && !keep_open
            && !plan.steps.is_empty()
            && plan.steps.iter().all(|step| step.done)
        {
            let state: Option<String> = self
                .connection
                .query_row(
                    "SELECT state FROM core_goals WHERE goal_id=?1",
                    [plan.goal_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?;
            if state.as_deref() == Some("open") {
                self.core_transition(plan.goal_id, "complete")?;
            }
        }
        Ok(())
    }
    fn core_set_all_steps(&mut self, id: i64, done: bool) -> Result<(), String> {
        let old = self.goal_plan(id)?;
        if old.steps.iter().all(|step| step.done == done) {
            return Ok(());
        }
        let mut plan = old.clone();
        for step in &mut plan.steps {
            step.done = done;
        }
        plan.current_step = None;
        let tx = self.connection.savepoint().map_err(db)?;
        record_step_changes(&tx, &old, &plan)?;
        plan.revision = plan
            .revision
            .checked_add(1)
            .ok_or("Goal revision limit reached")?;
        tx.execute("INSERT INTO goal_plans(goal_id,revision,payload) VALUES(?1,?2,?3) ON CONFLICT(goal_id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload", params![plan.goal_id,plan.revision,serde_json::to_string(&plan).map_err(|_| "Could not save steps")?]).map_err(db)?;
        tx.commit().map_err(db)
    }
    /// Open goals whose steps are all done are completed.
    pub fn core_settle_goals(&mut self) -> Result<(), String> {
        let ids: Vec<i64> = {
            let mut q = self
                .connection
                .prepare("SELECT goal_id FROM core_goals WHERE state='open' AND NOT EXISTS(SELECT 1 FROM core_goal_reopen_overrides r WHERE r.goal_id=core_goals.goal_id)")
                .map_err(db)?;
            let rows = q
                .query_map([], |r| r.get::<_, i64>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            rows
        };
        for id in ids {
            let plan = self.goal_plan(id)?;
            if plan.done_when.trim().is_empty()
                && !plan.steps.is_empty()
                && plan.steps.iter().all(|step| step.done)
            {
                self.core_transition(id, "complete")?;
            }
        }
        Ok(())
    }
    pub fn core_transition(&mut self, id: i64, action: &str) -> Result<(), String> {
        self.core_transition_with(id, action, true)
    }
    pub(crate) fn core_reopen_preserving_steps(&mut self, id: i64) -> Result<(), String> {
        let plan = self.goal_plan(id)?;
        if !plan.steps.is_empty() && plan.steps.iter().all(|s| s.done) {
            self.connection
                .execute(
                    "INSERT OR IGNORE INTO core_goal_reopen_overrides(goal_id) VALUES(?1)",
                    [id],
                )
                .map_err(db)?;
        }
        self.core_transition_with(id, "resume", false)
    }
    fn core_transition_with(
        &mut self,
        id: i64,
        action: &str,
        reset_steps_on_reopen: bool,
    ) -> Result<(), String> {
        let new_state = match action {
            "complete" => "completed",
            "defer" => "deferred",
            "resume" => "open",
            "delete" => "deleted",
            _ => return Err("Unknown goal action".into()),
        };
        if action == "complete" || (action == "resume" && reset_steps_on_reopen) {
            self.connection
                .execute(
                    "DELETE FROM core_goal_reopen_overrides WHERE goal_id=?1",
                    [id],
                )
                .map_err(db)?;
        }
        if action == "complete" {
            // Completing a goal completes every step inside it.
            self.core_set_all_steps(id, true)?;
        } else if action == "resume" {
            // Reopening a completed goal starts its steps over.
            let completed: bool = self
                .connection
                .query_row(
                    "SELECT g.status='completed' OR c.state='completed' FROM core_goals c JOIN goals g ON g.id=c.goal_id WHERE c.goal_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap_or(false);
            if completed && reset_steps_on_reopen {
                self.core_set_all_steps(id, false)?;
            }
        }
        let tx = self.connection.savepoint().map_err(db)?;
        let previous: String = tx.query_row("SELECT CASE WHEN g.status='completed' THEN 'completed' ELSE c.state END FROM core_goals c JOIN goals g ON g.id=c.goal_id WHERE c.goal_id=?1", [id], |r| r.get(0)).map_err(|_| "Goal no longer exists")?;
        if action == "delete" {
            tx.execute("DELETE FROM feedback WHERE decision_id IN(SELECT id FROM decisions WHERE goal_id=?1)", [id]).map_err(db)?;
            for table in [
                "activity",
                "decisions",
                "suggestions",
                "recommendation_history",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE goal_id=?1"), [id])
                    .map_err(db)?;
            }
            tx.execute("DELETE FROM goals WHERE id=?1", [id])
                .map_err(db)?;
        } else if previous != new_state {
            let now = Utc::now();
            tx.execute(
                "UPDATE core_goals SET state=?1 WHERE goal_id=?2",
                params![new_state, id],
            )
            .map_err(db)?;
            tx.execute(
                "UPDATE goals SET status=?1,completed_at=?2 WHERE id=?3",
                params![
                    if new_state == "completed" {
                        "completed"
                    } else {
                        "deferred"
                    },
                    if new_state == "completed" {
                        Some(now.to_rfc3339())
                    } else {
                        None
                    },
                    id
                ],
            )
            .map_err(db)?;
            if action == "complete" || (action == "resume" && previous == "completed") {
                let title: String = tx
                    .query_row("SELECT text FROM goals WHERE id=?1", [id], |r| r.get(0))
                    .map_err(db)?;
                tx.execute("INSERT INTO core_events(goal_id,step_id,title,kind,day,created_at,completion_source) VALUES(?1,'goal',?2,?3,?4,?5,'manual')", params![id,title,if action=="complete" {"completed"} else {"reopened"},now.with_timezone(&Local).date_naive().to_string(),now.to_rfc3339()]).map_err(db)?;
            }
        }
        tx.commit().map_err(db)?;
        if self.core_timer()?.is_some_and(|t| t.goal_id == id) {
            self.write_setting("core_timer", &Option::<Timer>::None)?;
        }
        Ok(())
    }
    pub fn core_today(&mut self, id: i64, include: bool, date: &str) -> Result<(), String> {
        day(date)?;
        let tx = self.connection.savepoint().map_err(db)?;
        if include {
            let state: String = tx
                .query_row("SELECT state FROM core_goals WHERE goal_id=?1", [id], |r| {
                    r.get(0)
                })
                .map_err(|_| "Goal no longer exists")?;
            if state != "open" {
                return Err("Resume this goal before adding it to Today".into());
            }
            tx.execute(
                "INSERT OR IGNORE INTO core_day_items(day,goal_id) VALUES(?1,?2)",
                params![date, id],
            )
            .map_err(db)?;
            tx.execute("INSERT INTO core_days(day,mode) VALUES(?1,'plan') ON CONFLICT(day) DO UPDATE SET mode='plan'", [date]).map_err(db)?;
        } else {
            tx.execute(
                "DELETE FROM core_day_items WHERE day=?1 AND goal_id=?2",
                params![date, id],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)
    }
    pub fn core_plan_goals(&mut self, ids: &[i64], date: &str) -> Result<(), String> {
        day(date)?;
        if ids.is_empty() {
            return Err("Choose at least one goal for Today".into());
        }
        let tx = self.connection.savepoint().map_err(db)?;
        for id in ids {
            let state: String = tx
                .query_row("SELECT state FROM core_goals WHERE goal_id=?1", [id], |r| {
                    r.get(0)
                })
                .map_err(|_| {
                    "A selected goal no longer exists. Review your selection and try again."
                })?;
            if state != "open" {
                return Err(
                    "A selected goal is no longer open. Review your selection and try again."
                        .into(),
                );
            }
            tx.execute(
                "INSERT OR IGNORE INTO core_day_items(day,goal_id) VALUES(?1,?2)",
                params![date, id],
            )
            .map_err(db)?;
        }
        tx.execute("INSERT INTO core_days(day,mode) VALUES(?1,'plan') ON CONFLICT(day) DO UPDATE SET mode='plan'", [date]).map_err(db)?;
        tx.commit().map_err(db)
    }
    pub fn core_plan_day(&mut self, date: &str, no_plan: bool) -> Result<(), String> {
        day(date)?;
        let tx = self.connection.savepoint().map_err(db)?;
        tx.execute("INSERT INTO core_days(day,mode) VALUES(?1,?2) ON CONFLICT(day) DO UPDATE SET mode=excluded.mode", params![date,if no_plan {"no_plan"} else {"plan"}]).map_err(db)?;
        if no_plan {
            tx.execute("DELETE FROM core_day_items WHERE day=?1", [date])
                .map_err(db)?;
        }
        tx.commit().map_err(db)
    }
    pub fn core_dismiss_carryover(&self, id: i64, date: &str) -> Result<(), String> {
        day(date)?;
        self.connection
            .execute(
                "INSERT OR IGNORE INTO core_carryover_dismissals(day,goal_id) VALUES(?1,?2)",
                params![date, id],
            )
            .map_err(db)?;
        Ok(())
    }
    pub fn core_create_goal(&mut self, title: &str, batch: &str, date: &str) -> Result<(), String> {
        let title = goals::text(title, true)?;
        day(date)?;
        if batch.is_empty()
            || batch.len() > 80
            || !batch
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("Invalid draft identifier".into());
        }
        let tx = self.connection.savepoint().map_err(db)?;
        let seen: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM core_quick_batches WHERE batch_id=?1)",
                [batch],
                |r| r.get(0),
            )
            .map_err(db)?;
        if seen {
            return Ok(());
        }
        let count: u32 = tx
            .query_row("SELECT COUNT(*) FROM goals", [], |r| r.get(0))
            .map_err(db)?;
        if count >= 500 {
            return Err("Keep up to 500 goals; remove an older goal before adding more".into());
        }
        tx.execute(
            "INSERT OR IGNORE INTO core_areas(title) VALUES('General')",
            [],
        )
        .map_err(db)?;
        tx.execute(
            "INSERT INTO goals(text,status,created_at) VALUES(?1,'deferred',?2)",
            params![title, Utc::now().to_rfc3339()],
        )
        .map_err(db)?;
        let id = tx.last_insert_rowid();
        tx.execute("INSERT INTO core_goal_analyses(goal_id) VALUES(?1)", [id])
            .map_err(db)?;
        tx.execute("INSERT INTO core_goals(goal_id,area_id,state) VALUES(?1,(SELECT id FROM core_areas WHERE title='General'),'open')", [id]).map_err(db)?;
        tx.execute(
            "INSERT INTO core_quick_batches(batch_id,goal_id) VALUES(?1,?2)",
            params![batch, id],
        )
        .map_err(db)?;
        // Quick entry always lands on Today; Today has no size limit.
        tx.execute(
            "INSERT INTO core_day_items(day,goal_id) VALUES(?1,?2)",
            params![date, id],
        )
        .map_err(db)?;
        tx.execute("INSERT INTO core_days(day,mode) VALUES(?1,'plan') ON CONFLICT(day) DO UPDATE SET mode='plan'", [date]).map_err(db)?;
        crate::onboarding_goal::goal_created(&tx)?;
        tx.commit().map_err(db)
    }
    pub fn core_goals_for_date(&self, date: &str) -> Result<Vec<i64>, String> {
        day(date)?;
        let mut query = self
            .connection
            .prepare("SELECT goal_id FROM core_day_items WHERE day=?1 ORDER BY rowid")
            .map_err(db)?;
        let rows = query.query_map([date], |r| r.get(0)).map_err(db)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db)
    }
    pub fn core_carryover(&self, date: &str) -> Result<Vec<i64>, String> {
        day(date)?;
        let mut query = self.connection.prepare("SELECT c.goal_id FROM core_goals c JOIN goals g ON g.id=c.goal_id WHERE c.state='open' AND g.status!='completed' AND EXISTS(SELECT 1 FROM core_day_items d WHERE d.goal_id=c.goal_id AND d.day<?1) AND NOT EXISTS(SELECT 1 FROM core_day_items d WHERE d.goal_id=c.goal_id AND d.day=?1) AND NOT EXISTS(SELECT 1 FROM core_carryover_dismissals x WHERE x.goal_id=c.goal_id AND x.day=?1) ORDER BY c.goal_id").map_err(db)?;
        let rows = query.query_map([date], |r| r.get(0)).map_err(db)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db)
    }
    pub fn core_timer(&self) -> Result<Option<Timer>, String> {
        self.read_setting("core_timer")
    }
    pub fn core_tick_at(&mut self, now: DateTime<Utc>) -> Result<(), String> {
        if let Some(mut timer) = self.core_timer()? {
            let tx = self.connection.savepoint().map_err(db)?;
            let duration = (now - timer.last_tick).num_seconds();
            // Count only live heartbeats, never app downtime, sleep or throttled gaps.
            if (1..=30).contains(&duration) {
                let mut cursor = timer.last_tick;
                for _ in 0..duration {
                    let date = cursor.with_timezone(&Local).date_naive().to_string();
                    tx.execute("INSERT INTO core_time(day,goal_id,seconds) VALUES(?1,?2,1) ON CONFLICT(day,goal_id) DO UPDATE SET seconds=seconds+1",params![date,timer.goal_id]).map_err(db)?;
                    cursor += Duration::seconds(1);
                }
            }
            if now >= timer.last_tick {
                timer.last_tick = if duration <= 30 {
                    timer.last_tick + Duration::seconds(duration)
                } else {
                    now
                };
                tx.execute(
                    "INSERT OR REPLACE INTO preferences(name,value) VALUES('core_timer',?1)",
                    [serde_json::to_string(&timer).map_err(|_| "Could not save timer")?],
                )
                .map_err(db)?;
            }
            tx.commit().map_err(db)?;
        }
        Ok(())
    }
    pub fn core_set_timer(&mut self, id: Option<i64>) -> Result<(), String> {
        self.core_tick_at(Utc::now())?;
        if let Some(id) = id {
            let state: String = self
                .connection
                .query_row("SELECT state FROM core_goals WHERE goal_id=?1", [id], |r| {
                    r.get(0)
                })
                .map_err(|_| "Goal no longer exists")?;
            if state != "open" {
                return Err("Resume the goal before starting its timer".into());
            }
        }
        self.write_setting(
            "core_timer",
            &id.map(|goal_id| Timer {
                goal_id,
                last_tick: Utc::now(),
            }),
        )
    }
    pub fn core_hourly(&self, date: &str) -> Result<Vec<HourTime>, String> {
        use chrono::{TimeZone, Timelike};
        let start = Local
            .from_local_datetime(&day(date)?.and_hms_opt(0, 0, 0).ok_or("Invalid day")?)
            .earliest()
            .ok_or("Invalid day")?
            .timestamp_millis();
        let end = start + 24 * 3_600_000;
        let mut q = self
            .connection
            .prepare("SELECT goal_id,started_at,ended_at FROM usage_intervals WHERE match_confidence>0 AND julianday(ended_at)>julianday(?1) AND julianday(started_at)<julianday(?2)")
            .map_err(db)?;
        let rows = q
            .query_map(
                params![
                    DateTime::from_timestamp_millis(start).map(|t| t.to_rfc3339()),
                    DateTime::from_timestamp_millis(end).map(|t| t.to_rfc3339())
                ],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let mut totals = std::collections::BTreeMap::<(u8, i64), i64>::new();
        for (goal, from, to) in rows {
            let (Ok(from), Ok(to)) = (
                DateTime::parse_from_rfc3339(&from),
                DateTime::parse_from_rfc3339(&to),
            ) else {
                continue;
            };
            let mut cursor = from.timestamp_millis().max(start);
            let last = to.timestamp_millis().min(end);
            while cursor < last {
                let local = DateTime::from_timestamp_millis(cursor)
                    .ok_or("Invalid sample time")?
                    .with_timezone(&Local);
                let hour_start = local
                    .with_minute(0)
                    .and_then(|t| t.with_second(0))
                    .and_then(|t| t.with_nanosecond(0))
                    .ok_or("Invalid hour")?;
                let next = (hour_start.timestamp_millis() + 3_600_000).min(last);
                *totals.entry((local.hour() as u8, goal)).or_default() += next - cursor;
                cursor = next;
            }
        }
        if let Some((hour, id)) = self.onboarding_hour(date)? {
            *totals.entry((hour, id)).or_default() += 300_000;
        }
        Ok(totals
            .into_iter()
            .map(|((hour, goal_id), ms)| HourTime {
                hour,
                goal_id,
                seconds: (ms / 1000) as u64,
            })
            .filter(|item| item.seconds > 0)
            .collect())
    }
    pub(crate) fn step_times(&self, goal: i64, date: &str) -> Result<Vec<StepTime>, String> {
        let mut q = self
            .connection
            .prepare("SELECT step_id,milliseconds/1000 FROM goal_relevant_step_daily WHERE goal_id=?1 AND day=?2 ORDER BY step_id")
            .map_err(db)?;
        let rows = q
            .query_map(params![goal, date], |r| {
                Ok(StepTime {
                    step_id: r.get(0)?,
                    seconds: r.get(1)?,
                })
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(rows)
    }
    pub(crate) fn core_progress(&self, date: &str) -> Result<DayProgress, String> {
        let mut q=self.connection.prepare("SELECT e.title FROM core_events e WHERE e.day=?1 AND e.kind='completed' AND e.id=(SELECT MAX(x.id) FROM core_events x WHERE x.goal_id=e.goal_id AND x.step_id=e.step_id AND x.day=e.day) ORDER BY e.id").map_err(db)?;
        let completed = q
            .query_map([date], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let mut q=self.connection.prepare("WITH times AS (SELECT goal_id,seconds,0 tracked_seconds FROM core_time WHERE day=?1 UNION ALL SELECT goal_id,0,SUM(milliseconds)/1000 FROM usage_daily WHERE day=?1 GROUP BY goal_id) SELECT t.goal_id,g.text,a.title,SUM(t.seconds),SUM(t.tracked_seconds) FROM times t JOIN goals g ON g.id=t.goal_id JOIN core_goals c ON c.goal_id=g.id JOIN core_areas a ON a.id=c.area_id GROUP BY t.goal_id ORDER BY SUM(t.seconds)+SUM(t.tracked_seconds) DESC,g.id").map_err(db)?;
        let goals = q
            .query_map([date], |r| {
                Ok(GoalTime {
                    goal_id: r.get(0)?,
                    title: r.get(1)?,
                    area: r.get(2)?,
                    seconds: r.get(3)?,
                    tracked_seconds: r.get(4)?,
                    relevant_seconds: self
                        .relevant_seconds(r.get(0)?, Some(date))
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    steps: self
                        .step_times(r.get(0)?, date)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                })
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(DayProgress {
            day: date.into(),
            completed_goals:self.connection.prepare("SELECT e.title FROM core_events e WHERE e.day=?1 AND e.step_id='goal' AND e.kind='completed' AND e.id=(SELECT MAX(x.id) FROM core_events x WHERE x.goal_id=e.goal_id AND x.step_id='goal' AND x.day=e.day)").map_err(db)?.query_map([date],|r|r.get(0)).map_err(db)?.collect::<Result<Vec<_>,_>>().map_err(db)?,
            completed,
            goals,
        })
    }
    pub fn core_snapshot(&self, anchor: Option<&str>) -> Result<CoreSnapshot, String> {
        self.core_snapshot_period(anchor, None)
    }
    pub fn core_snapshot_period(
        &self,
        anchor: Option<&str>,
        period: Option<&str>,
    ) -> Result<CoreSnapshot, String> {
        self.core_snapshot_with_progress(anchor, period, true)
    }
    /// Background decisions need current lanes, not a rebuilt seven-day Progress report.
    pub(crate) fn core_work_snapshot(&self) -> Result<CoreSnapshot, String> {
        self.core_snapshot_with_progress(None, None, false)
    }
    fn core_snapshot_with_progress(
        &self,
        anchor: Option<&str>,
        period: Option<&str>,
        progress: bool,
    ) -> Result<CoreSnapshot, String> {
        self.adopt_core_goals()?;
        let today = Local::now().date_naive();
        let date = today.to_string();
        let anchor = anchor.map(day).transpose()?.unwrap_or(today);
        let days = if progress {
            period_days(period, anchor)?
        } else {
            0
        };
        let mut q = self
            .connection
            .prepare("SELECT id,title FROM core_areas ORDER BY title COLLATE NOCASE")
            .map_err(db)?;
        let areas = q
            .query_map([], |r| {
                Ok(Area {
                    id: r.get(0)?,
                    title: r.get(1)?,
                })
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let mut q=self.connection.prepare("SELECT g.id,g.text,CASE WHEN g.status='completed' THEN 'completed' ELSE c.state END,c.area_id,COALESCE((SELECT SUM(seconds) FROM core_time WHERE goal_id=g.id),0),m.due_at,m.priority,COALESCE(m.description,''),g.created_at,g.completed_at FROM goals g JOIN core_goals c ON c.goal_id=g.id LEFT JOIN core_goal_details m ON m.goal_id=g.id ORDER BY CASE c.state WHEN 'open' THEN 0 WHEN 'deferred' THEN 1 ELSE 2 END,g.id DESC").map_err(db)?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let memory_ready:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='personal_memory_migrations')",[],|r|r.get(0)).map_err(db)?;
        let memory_ready = memory_ready
            && self
                .connection
                .query_row(
                    "SELECT COALESCE(MAX(version),0)>=3 FROM personal_memory_migrations",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(db)?;
        let mut goals = rows
            .into_iter()
            .map(
                |(
                    id,
                    title,
                    status,
                    area_id,
                    focused_seconds,
                    due_at,
                    priority,
                    description,
                    created_at,
                    completed_at,
                )| {
                    Ok(CoreGoal {
                        id,
                        title,
                        status,
                        area_id,
                        plan: self.goal_plan(id)?,
                        relevant_seconds: self.relevant_seconds(id,None)?,
                        analysis: self.goal_analysis_view(id)?,
                        focused_seconds,
                        tracked_seconds: self.connection.query_row("SELECT COALESCE(SUM(milliseconds),0)/1000 FROM usage_daily WHERE goal_id=?1", [id], |r| r.get(0)).map_err(db)?,
                        due_at,
                        priority,
                        description,
                        created_at,
                        completed_at,
                        estimated_minutes: self.connection.query_row("SELECT minutes FROM ai_goal_estimates WHERE goal_id=?1",[id],|r| r.get::<_, i64>(0)).optional().map_err(db)?,
                        work_context: if memory_ready {self.work_context(id)?} else {None},
                    })
                },
            )
            .collect::<Result<Vec<_>, String>>()?;
        let order: Vec<i64> = self.read_setting("core_goal_order")?.unwrap_or_default();
        goals.sort_by_key(|goal| {
            order
                .iter()
                .position(|id| *id == goal.id)
                .unwrap_or(usize::MAX)
        });
        let mut q = self
            .connection
            .prepare("SELECT goal_id FROM core_day_items WHERE day=?1 ORDER BY rowid")
            .map_err(db)?;
        let items = q
            .query_map([&date], |r| r.get(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let mode = self
            .connection
            .query_row("SELECT mode FROM core_days WHERE day=?1", [&date], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(db)?
            .unwrap_or_else(|| "unset".into());
        let week = (0..days)
            .map(|offset| self.core_progress(&(anchor - Duration::days(offset)).to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CoreSnapshot {
            onboarding_goal_id: self.onboarding_goal_id()?,
            date: date.clone(),
            selected_date: anchor.to_string(),
            date_goals: self.core_goals_for_date(&anchor.to_string())?,
            carryover: self.core_carryover(&date)?,
            day_mode: mode,
            today: items,
            areas,
            goals,
            summary: if progress {
                self.core_progress(&date)?
            } else {
                DayProgress {
                    day: date.clone(),
                    completed: vec![],
                    completed_goals: vec![],
                    goals: vec![],
                }
            },
            week,
            hourly: if progress {
                self.core_hourly(&anchor.to_string())?
            } else {
                vec![]
            },
            timer: self.core_timer()?,
            focused_goal_id: self.goal()?.map(|g| g.id),
            preferences: self.read_setting("core_preferences")?.unwrap_or_default(),
        })
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command arguments are the frontend IPC contract"
)]
#[tauri::command(async)]
pub fn finish_core_setup(
    state: State<AppState>,
    app: AppHandle,
    name: String,
    drafts: Vec<GoalDraft>,
    batch: String,
    allow_tracking: bool,
    email: Option<String>,
    avatar: Option<crate::profile::AvatarPreferences>,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err("Enter your name, up to 120 characters".into());
    }
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    let mut settings = inner.storage.user_settings()?;
    if settings.onboarding.completed {
        return Err("Setup is already complete".into());
    }
    settings.profile.name = name.into();
    if let Some(email) = email {
        settings.profile.email = email;
    }
    if let Some(avatar) = avatar {
        settings.profile.avatar = avatar;
    }
    crate::profile::validate_profile(&mut settings.profile, false)?;
    if !drafts.is_empty() {
        inner.storage.core_add_goals(drafts, &batch)?;
    }
    inner.storage.start_onboarding_goal()?;
    let snapshot = inner.storage.core_snapshot(None)?;
    let open: Vec<i64> = snapshot
        .goals
        .iter()
        .filter(|g| g.status == "open")
        .map(|g| g.id)
        .collect();
    for id in open.iter() {
        inner.storage.core_today(*id, true, &snapshot.date)?;
    }
    settings.onboarding.completed = true;
    settings.onboarding.step = 3;
    settings.onboarding.tracking_consent = allow_tracking;
    crate::autostart::sync(settings.autostart)?;
    inner.storage.write_setting("user_settings", &settings)?;
    inner.status.tracking = allow_tracking;
    inner
        .storage
        .write_setting("tracking_requested", &allow_tracking)?;
    crate::tracking::configure_goal_monitoring(&mut inner, allow_tracking)?;
    inner.collector = crate::collector::create(inner.status.demo);
    inner.usage = Default::default();
    inner.activity_state.stop(false);
    crate::buddy::sync(&app, &mut inner)
}

#[tauri::command(async)]
pub fn save_core_identity(
    state: State<AppState>,
    app: AppHandle,
    name: String,
    autostart: bool,
    avatar: Option<crate::profile::AvatarPreferences>,
    email: Option<String>,
) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    let mut settings = inner.storage.user_settings()?;
    settings.profile.name = name.trim().into();
    if let Some(email) = email {
        settings.profile.email = email;
    }
    if let Some(avatar) = avatar {
        settings.profile.avatar = avatar;
    }
    if settings.profile.name.is_empty() {
        return Err("Enter your name".into());
    }
    crate::profile::validate_profile(&mut settings.profile, false)?;
    crate::autostart::sync(autostart)?;
    settings.autostart = autostart;
    inner.storage.write_setting("user_settings", &settings)?;
    crate::buddy::sync(&app, &mut inner)
}
pub(crate) fn mutate(
    state: &AppState,
    app: &AppHandle,
    work: impl FnOnce(&mut Storage) -> Result<(), String>,
) -> Result<CoreSnapshot, String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    let focus = inner.storage.goal()?.map(|g| g.id);
    work(&mut inner.storage)?;
    inner.storage.ensure_today_focus()?;
    if focus != inner.storage.goal()?.map(|g| g.id) {
        inner.status.tracking = crate::tracking::requested(&inner.storage)?;
        inner.collector = crate::collector::create(inner.status.demo);
        inner.usage = Default::default();
        inner.activity_state.stop(false);
    }
    inner.privacy_revision += 1;
    inner.goal_matching.clear();
    inner.companion.invalidate();
    inner.buddy.clear();
    inner.last_analysis = None;
    crate::frontend_events::changed(
        app,
        &[
            crate::frontend_events::Resource::Dashboard,
            crate::frontend_events::Resource::Workspace,
            crate::frontend_events::Resource::Activity,
            crate::frontend_events::Resource::Coaching,
        ],
        false,
    );
    crate::companion::offer_onboarding(app, &mut inner)?;
    crate::buddy::sync(app, &mut inner)?;
    inner.storage.core_snapshot(None)
}
#[tauri::command(async)]
pub fn get_core_snapshot(
    state: State<AppState>,
    anchor: Option<String>,
    period: Option<String>,
) -> Result<CoreSnapshot, String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.storage.core_settle_goals()?;
    inner
        .storage
        .core_snapshot_period(anchor.as_deref(), period.as_deref())
}
#[tauri::command(async)]
pub fn visit_core_progress(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    if !inner.storage.user_settings()?.onboarding.completed {
        return Ok(());
    }
    inner.storage.visit_onboarding_progress()?;
    crate::companion::offer_onboarding(&app, &mut inner)?;
    crate::buddy::sync(&app, &mut inner)
}

#[tauri::command(async)]
pub fn add_core_goals(
    state: State<AppState>,
    app: AppHandle,
    drafts: Vec<GoalDraft>,
    batch: String,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |s| s.core_add_goals(drafts, &batch))
}
#[tauri::command(async)]
pub fn create_core_goal(
    state: State<AppState>,
    app: AppHandle,
    title: String,
    batch: String,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |s| {
        s.core_create_goal(&title, &batch, &Local::now().date_naive().to_string())
    })
}
#[tauri::command(async)]
pub fn save_core_goal(
    state: State<AppState>,
    app: AppHandle,
    title: String,
    area: String,
    plan: GoalPlan,
    details: Option<GoalDetails>,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |s| {
        s.core_save_goal_details(&title, &area, plan, details)
    })
}
/// Changes only the priority of a goal; other details are kept.
#[tauri::command(async)]
pub fn set_core_goal_priority(
    state: State<AppState>,
    app: AppHandle,
    id: i64,
    priority: Option<String>,
) -> Result<CoreSnapshot, String> {
    GoalDetails {
        priority: priority.clone(),
        ..Default::default()
    }
    .validate()?;
    mutate(&state, &app, |s| {
        let exists: bool = s
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM core_goals WHERE goal_id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !exists {
            return Err("Goal not found".into());
        }
        s.connection
            .execute(
                "INSERT INTO core_goal_details(goal_id,due_at,priority,description) VALUES(?1,NULL,?2,'') ON CONFLICT(goal_id) DO UPDATE SET priority=excluded.priority",
                params![id, priority],
            )
            .map_err(db)?;
        Ok(())
    })
}
/// Sets or clears the expected minutes for a goal.
#[tauri::command(async)]
pub fn set_core_goal_estimate(
    state: State<AppState>,
    app: AppHandle,
    id: i64,
    minutes: Option<i64>,
) -> Result<CoreSnapshot, String> {
    if minutes.is_some_and(|m| !(1..=100_000).contains(&m)) {
        return Err("Use between 1 and 100,000 minutes".into());
    }
    mutate(&state, &app, |s| {
        let exists: bool = s
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM core_goals WHERE goal_id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !exists {
            return Err("Goal not found".into());
        }
        match minutes {
            Some(minutes) => s.connection.execute("INSERT INTO ai_goal_estimates(goal_id,minutes) VALUES(?1,?2) ON CONFLICT(goal_id) DO UPDATE SET minutes=excluded.minutes", params![id, minutes]).map_err(db)?,
            None => s
                .connection
                .execute("DELETE FROM ai_goal_estimates WHERE goal_id=?1", [id])
                .map_err(db)?,
        };
        Ok(())
    })
}
#[tauri::command(async)]
pub fn dismiss_core_carryover(
    state: State<AppState>,
    app: AppHandle,
    id: i64,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |s| {
        s.core_dismiss_carryover(id, &Local::now().date_naive().to_string())
    })
}
#[tauri::command(async)]
pub fn transition_core_goal(
    state: State<AppState>,
    app: AppHandle,
    id: i64,
    action: String,
    confirmed: bool,
) -> Result<CoreSnapshot, String> {
    if matches!(action.as_str(), "delete" | "complete") && !confirmed {
        return Err("Confirm this goal action first".into());
    }
    let mut newly_completed = false;
    let snapshot = mutate(&state, &app, |s| {
        let completed: bool = s
            .connection
            .query_row(
                "SELECT status='completed' FROM goals WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| "Goal no longer exists")?;
        s.core_transition(id, &action)?;
        newly_completed = action == "complete" && !completed;
        Ok(())
    })?;
    if newly_completed {
        if let Some(goal) = snapshot.goals.iter().find(|g| g.id == id) {
            let payload = serde_json::json!({"goalId":id.to_string(),"completedAt":goal.completed_at,"activeMinutes":goal.relevant_seconds as f64 / 60.});
            let _ = app.emit("buddy://goal-completed", &payload);
            crate::companion::emit(&app, "goal.completed", payload);
        }
    }
    Ok(snapshot)
}
#[tauri::command(async)]
pub fn set_core_today(
    state: State<AppState>,
    app: AppHandle,
    id: i64,
    include: bool,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |s| {
        s.core_today(id, include, &Local::now().date_naive().to_string())
    })
}
#[tauri::command(async)]
pub fn reorder_core_goals(
    state: State<AppState>,
    app: AppHandle,
    ids: Vec<i64>,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |storage| storage.core_reorder(ids))
}
#[tauri::command(async)]
pub fn plan_core_goals(
    state: State<AppState>,
    app: AppHandle,
    ids: Vec<i64>,
    date: Option<String>,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |storage| {
        storage.core_plan_goals(
            &ids,
            date.as_deref()
                .unwrap_or(&Local::now().date_naive().to_string()),
        )
    })
}
#[tauri::command(async)]
pub fn plan_core_day(
    state: State<AppState>,
    app: AppHandle,
    no_plan: bool,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |s| {
        s.core_plan_day(&Local::now().date_naive().to_string(), no_plan)
    })
}
#[tauri::command(async)]
pub fn set_core_timer(
    state: State<AppState>,
    app: AppHandle,
    id: Option<i64>,
) -> Result<CoreSnapshot, String> {
    mutate(&state, &app, |storage| {
        storage.core_set_timer(id)?;
        if let Some(id) = id {
            if storage.goal()?.map(|goal| goal.id) != Some(id) {
                storage.transition_goal(id, "resume")?;
            }
            storage.mark_explicit_focus(id)?;
        }
        Ok(())
    })
}
#[tauri::command(async)]
pub fn tick_core_timer(state: State<AppState>) -> Result<CoreSnapshot, String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.storage.core_tick_at(Utc::now())?;
    inner.storage.core_snapshot(None)
}
/// The visible stretch switch updates both historical preference records atomically.
fn persist_preferences(
    storage: &mut Storage,
    preferences: &CorePreferences,
) -> Result<crate::companion::engine::Preferences, String> {
    let mut companion: crate::companion::engine::Preferences = storage
        .read_setting("companion_preferences")?
        .unwrap_or_default();
    companion.movement_reminder = preferences.movement_reminders;
    let tx = storage.connection.savepoint().map_err(db)?;
    for (name, value) in [
        ("core_preferences", serde_json::to_string(preferences)),
        ("companion_preferences", serde_json::to_string(&companion)),
    ] {
        tx.execute("INSERT INTO preferences(name,value) VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET value=excluded.value", params![name, value.map_err(|_| "Could not serialize settings")?]).map_err(db)?;
    }
    tx.commit().map_err(db)?;
    Ok(companion)
}

#[tauri::command(async)]
pub fn set_core_preferences(
    state: State<AppState>,
    app: AppHandle,
    mut preferences: CorePreferences,
) -> Result<(), String> {
    preferences.working_days.sort_unstable();
    preferences.working_days.dedup();
    if preferences.working_days.iter().any(|d| *d > 6)
        || !(5..=240).contains(&preferences.movement_interval_minutes)
        || preferences.vision_model.len() > 160
        || preferences.vision_model.chars().any(char::is_control)
    {
        return Err("Invalid workspace settings".into());
    }
    preferences.vision_model = preferences.vision_model.trim().into();
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    let previous: CorePreferences = inner
        .storage
        .read_setting("core_preferences")?
        .unwrap_or_default();
    let companion = persist_preferences(&mut inner.storage, &preferences)?;
    if previous.ai_connection_mode != preferences.ai_connection_mode {
        #[cfg(not(test))]
        crate::shared_ai::activate(preferences.ai_connection_mode);
        inner.mark_consent_change();
        inner.status.anthropic_configured = crate::http::configured("ANTHROPIC_API_KEY");
        inner.status.web_search_configured = crate::http::configured("ANTHROPIC_API_KEY");
    }
    inner.companion.view.preferences = companion;
    crate::frontend_events::changed(
        &app,
        &[
            crate::frontend_events::Resource::Workspace,
            crate::frontend_events::Resource::Dashboard,
            crate::frontend_events::Resource::Services,
        ],
        false,
    );
    crate::buddy::sync(&app, &mut inner)?;
    Ok(())
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
