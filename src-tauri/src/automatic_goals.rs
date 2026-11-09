use crate::{
    commands::{AppState, Inner},
    core::CoreGoal,
    goal_analysis::{self, AnalysisInput, AnalysisResult},
    storage::Storage,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AnalysisView {
    pub state: String,
    pub result: Option<AnalysisResult>,
    pub message: Option<String>,
}
impl Storage {
    pub fn initialize_goal_automation(&self) -> Result<(), String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS core_goal_analyses(goal_id INTEGER PRIMARY KEY REFERENCES goals(id) ON DELETE CASCADE,state TEXT NOT NULL DEFAULT 'queued',result TEXT,message TEXT,attempts INTEGER NOT NULL DEFAULT 0);").map_err(|_| "Goal suggestions unavailable".to_string())?;
        // Only initialization recovers a job interrupted by a previous process.
        self.connection
            .execute(
                "UPDATE core_goal_analyses SET state='queued' WHERE state='working'",
                [],
            )
            .map_err(|_| "Goal suggestions unavailable".to_string())?;
        Ok(())
    }
    pub fn goal_analysis_view(&self, id: i64) -> Result<AnalysisView, String> {
        let row: Option<(String, Option<String>, Option<String>)> = self
            .connection
            .query_row(
                "SELECT state,result,message FROM core_goal_analyses WHERE goal_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| "Goal suggestions unavailable".to_string())?;
        match row {
            None => Ok(AnalysisView::default()),
            Some((state, result, message)) => {
                Ok(AnalysisView {
                    result: result
                        .map(|text| {
                            serde_json::from_str(&text)
                                .map_err(|_| "Saved goal suggestions unavailable".to_string())
                        })
                        .transpose()?,
                    message: if state == "queued" && !self.ai_preferences()?.enabled {
                        Some("AI assistance is paused. Enable it in Settings to prepare suggestions.".into())
                    } else {
                        message
                    },
                    state,
                })
            }
        }
    }
    pub fn retry_goal_analysis(&self, id: i64) -> Result<(), String> {
        self.goal_plan(id)?;
        self.connection.execute("INSERT INTO core_goal_analyses(goal_id,state) VALUES(?1,'queued') ON CONFLICT(goal_id) DO UPDATE SET state='queued',message=NULL,attempts=0", [id]).map_err(|_| "Could not retry suggestions".to_string())?;
        Ok(())
    }
    pub fn ensure_today_focus(&mut self) -> Result<Option<i64>, String> {
        let current = self.goal()?.map(|goal| goal.id);
        let day = chrono::Local::now().date_naive().to_string();
        let valid: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM core_day_items d JOIN core_goals c ON c.goal_id=d.goal_id JOIN goals g ON g.id=c.goal_id WHERE d.day=?1 AND d.goal_id=?2 AND c.state='open' AND g.status!='completed')",params![day,current],|r|r.get(0)).map_err(|_| "Today goals unavailable")?;
        if valid {
            return Ok(current);
        }
        if let Some(id) = current {
            // Clearing focus must not set an otherwise open goal aside.
            self.connection
                .execute(
                    "UPDATE goals SET status='deferred' WHERE id=?1 AND status='active'",
                    [id],
                )
                .map_err(|_| "Could not change focus")?;
        }
        let next:Option<i64> = self.connection.query_row("SELECT g.id FROM core_day_items d JOIN core_goals c ON c.goal_id=d.goal_id JOIN goals g ON g.id=c.goal_id LEFT JOIN core_goal_details m ON m.goal_id=g.id WHERE d.day=?1 AND c.state='open' AND g.status!='completed' AND NOT EXISTS(SELECT 1 FROM core_days WHERE day=?1 AND mode='no_plan') ORDER BY CASE m.priority WHEN 'high' THEN 3 WHEN 'medium' THEN 2 WHEN 'low' THEN 1 ELSE 0 END DESC,d.rowid LIMIT 1",[day],|r|r.get(0)).optional().map_err(|_| "Today goals unavailable")?;
        let next = if self.read_setting::<Vec<i64>>("core_goal_order")?.is_some() {
            let snapshot = self.core_snapshot(None)?;
            snapshot
                .goals
                .iter()
                .find(|g| snapshot.today.contains(&g.id) && g.status == "open")
                .map(|g| g.id)
        } else {
            next
        };
        if let Some(id) = next {
            self.transition_goal(id, "resume")?;
        }
        Ok(next)
    }
}
pub fn sync_focus(inner: &mut Inner) -> Result<(), String> {
    if inner.storage.explicit_focus()?.is_some()
        || (inner.goal_matching.view.goal_id.is_some()
            && inner.storage.goal()?.map(|g| g.id) == inner.goal_matching.view.goal_id)
    {
        return Ok(());
    }
    // Claude chooses among all open goals. Do not replace its choice with Today ordering.
    if inner.storage.ai_preferences()?.automatic_goal_matching {
        return Ok(());
    }
    let before = inner.storage.goal()?.map(|g| g.id);
    let after = inner.storage.ensure_today_focus()?;
    if before != after {
        inner.status.tracking = crate::tracking::requested(&inner.storage)?;
        inner.usage = Default::default();
        inner.activity_state.stop(false);
        inner.collector = crate::collector::create(inner.status.demo);
        inner.privacy_revision += 1;
        inner.companion.invalidate();
        inner.buddy.clear();
        inner.last_analysis = None;
    }
    Ok(())
}
#[tauri::command(async)]
pub fn select_core_goal(id: i64, app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    focus_goal_inner(&mut inner, id)?;
    crate::buddy::sync(&app, &mut inner)
}
pub(crate) fn focus_goal_inner(inner: &mut crate::commands::Inner, id: i64) -> Result<(), String> {
    let snapshot = inner.storage.core_snapshot(None)?;
    if !snapshot.today.contains(&id)
        || !snapshot
            .goals
            .iter()
            .any(|g| g.id == id && g.status == "open")
    {
        return Err("Choose an open goal from Today".into());
    }
    if snapshot.focused_goal_id != Some(id) {
        inner.storage.transition_goal(id, "resume")?;
    }
    inner.storage.mark_explicit_focus(id)?;
    inner.status.tracking = crate::tracking::requested(&inner.storage)?;
    inner.usage = Default::default();
    inner.activity_state.stop(false);
    inner.buddy.clear();
    inner.privacy_revision += 1;
    inner.companion.invalidate();
    Ok(())
}
pub fn input(goal: &CoreGoal) -> AnalysisInput {
    AnalysisInput {
        goal_id: goal.id.to_string(),
        title: goal.title.clone(),
        description: Some(goal.description.clone()),
        due_at: goal.due_at.clone(),
        priority: goal.priority.clone(),
        existing_steps: goal.plan.steps.clone(),
        user_context_ref: None,
    }
}
/// Durable jobs are created only for newly added goals. Existing goals are never
/// uploaded retroactively when upgrading. Manual chat takes the same provider lock.
pub async fn run_pending(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(_guard) = state.companion_request.try_lock() else {
        return;
    };
    let job = (|| -> Result<Option<AnalysisInput>, String> {
        let inner = state
            .inner
            .lock()
            .map_err(|_| "Application state unavailable")?;
        if !inner.storage.user_settings()?.onboarding.completed
            || !inner.storage.ai_preferences()?.enabled
        {
            return Ok(None);
        }
        let queued:bool=inner.storage.connection.query_row("SELECT EXISTS(SELECT 1 FROM core_goal_analyses a JOIN core_goals c ON c.goal_id=a.goal_id WHERE a.state='queued' AND c.state='open')",[],|r|r.get(0)).map_err(|_| "Goal suggestions unavailable")?;
        if !queued {
            return Ok(None);
        }
        let snapshot = inner.storage.core_snapshot(None)?;
        let Some(goal) = snapshot
            .goals
            .iter()
            .find(|g| g.status == "open" && g.analysis.state == "queued")
        else {
            return Ok(None);
        };
        inner.storage.connection.execute("UPDATE core_goal_analyses SET state='working',attempts=attempts+1 WHERE goal_id=?1 AND state='queued'", [goal.id]).map_err(|_| "Goal suggestions unavailable")?;
        let mut input = input(goal);
        input.user_context_ref = Some(
            crate::observation::personal_context(&inner, &goal.title, Some(goal.id)).to_string(),
        );
        Ok(Some(input))
    })();
    let Ok(Some(input)) = job else { return };
    let id = input.goal_id.parse::<i64>().unwrap_or_default();
    let _ = app.emit("buddy://goal-analysis", id);
    let result = goal_analysis::analyze_goal(input, true, &state).await;
    if let Ok(inner) = state.inner.lock() {
        // If a user explicitly retried or removed this goal, discard the older job.
        let working: bool = inner.storage.connection.query_row("SELECT EXISTS(SELECT 1 FROM core_goal_analyses WHERE goal_id=?1 AND state='working')", [id], |r| r.get(0)).unwrap_or(false);
        if working {
            match result {
                Ok(result) => {
                    if let Ok(payload) = serde_json::to_string(&result) {
                        let _ = inner.storage.connection.execute("UPDATE core_goal_analyses SET state='ready',result=?2,message=NULL WHERE goal_id=?1", params![id,payload]);
                    }
                }
                Err(error) => {
                    let message = goal_analysis::user_error("automatic_goal", &error);
                    let attempts: u32 = inner
                        .storage
                        .connection
                        .query_row(
                            "SELECT attempts FROM core_goal_analyses WHERE goal_id=?1",
                            [id],
                            |r| r.get(0),
                        )
                        .unwrap_or(3);
                    if (error.contains("changed") || error.contains("discarded")) && attempts < 3 {
                        let _ = inner.storage.connection.execute(
                            "UPDATE core_goal_analyses SET state='queued' WHERE goal_id=?1",
                            [id],
                        );
                    } else {
                        let _ = inner.storage.connection.execute(
                        "UPDATE core_goal_analyses SET state='failed',message=?2 WHERE goal_id=?1",
                        params![id, message],
                    );
                    }
                }
            }
        }
    }
    let _ = app.emit("buddy://goal-analysis", id);
}
#[tauri::command(async)]
pub fn retry_core_goal_analysis(id: i64, state: State<AppState>) -> Result<(), String> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| "Application state unavailable")?;
    inner.storage.retry_goal_analysis(id)?;
    inner.privacy_revision += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_focus_moves_first_and_next_goal_follows_the_saved_queue() {
        let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
        s.core_add_goals(
            (1..=5)
                .map(|i| crate::core::GoalDraft {
                    title: format!("Goal {i}"),
                    area: "Work".into(),
                    steps: vec![],
                })
                .collect(),
            "queue-test",
        )
        .unwrap();
        let ids: Vec<_> = s
            .core_snapshot(None)
            .unwrap()
            .goals
            .iter()
            .map(|g| g.id)
            .collect();
        for id in &ids {
            s.core_today(*id, true, &chrono::Local::now().date_naive().to_string())
                .unwrap();
        }
        s.core_reorder(vec![ids[4], ids[2], ids[0], ids[1], ids[3]])
            .unwrap();
        s.transition_goal(ids[2], "resume").unwrap();
        s.mark_explicit_focus(ids[2]).unwrap();
        assert_eq!(s.core_snapshot(None).unwrap().goals[0].id, ids[2]);
        s.core_transition(ids[2], "complete").unwrap();
        assert_eq!(s.ensure_today_focus().unwrap(), Some(ids[4]));
    }
    #[test]
    fn new_goals_queue_once_and_focus_without_starting_a_timer() {
        let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
        let day = chrono::Local::now().date_naive().to_string();
        storage
            .core_create_goal("Ship a landing page", "one", &day)
            .unwrap();
        storage
            .core_create_goal("Ship a landing page", "one", &day)
            .unwrap();
        let snapshot = storage.core_snapshot(None).unwrap();
        assert_eq!(snapshot.goals.len(), 1);
        assert_eq!(snapshot.goals[0].analysis.state, "queued");
        assert!(snapshot.timer.is_none());
        let id = snapshot.goals[0].id;
        assert_eq!(storage.ensure_today_focus().unwrap(), Some(id));
        storage.core_transition(id, "complete").unwrap();
        assert_eq!(storage.ensure_today_focus().unwrap(), None);
    }
    #[test]
    fn cached_results_survive_step_edits_and_delete_with_the_goal() {
        let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
        storage
            .core_create_goal(
                "Draft an article",
                "article",
                &chrono::Local::now().date_naive().to_string(),
            )
            .unwrap();
        let id = storage.core_snapshot(None).unwrap().goals[0].id;
        storage
            .connection
            .execute(
                "UPDATE core_goal_analyses SET state='ready',result=?2 WHERE goal_id=?1",
                params![
                    id,
                    serde_json::to_string(&AnalysisResult::default()).unwrap()
                ],
            )
            .unwrap();
        let mut plan = storage.goal_plan(id).unwrap();
        plan.steps.push(crate::goals::Step {
            id: "a".into(),
            text: "Outline".into(),
            done: false,
        });
        storage
            .core_save_goal_details("Draft an article", "General", plan, None)
            .unwrap();
        assert_eq!(storage.goal_analysis_view(id).unwrap().state, "ready");
        storage.core_transition(id, "delete").unwrap();
        assert!(storage.goal_analysis_view(id).unwrap().state.is_empty());
    }
    #[test]
    fn interrupted_analysis_recovers_but_legacy_goals_are_not_automatically_uploaded() {
        let path = std::env::temp_dir().join(format!(
            "buddy-automation-{}.sqlite",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let id;
        {
            let mut storage = Storage::open(&path).unwrap();
            let legacy = storage.set_goal("An existing personal goal").unwrap();
            assert!(storage
                .goal_analysis_view(legacy.id)
                .unwrap()
                .state
                .is_empty());
            storage
                .core_create_goal(
                    "Sumi onboarding",
                    "new",
                    &chrono::Local::now().date_naive().to_string(),
                )
                .unwrap();
            id = storage
                .core_snapshot(None)
                .unwrap()
                .goals
                .iter()
                .find(|g| g.title == "Sumi onboarding")
                .unwrap()
                .id;
            storage
                .connection
                .execute(
                    "UPDATE core_goal_analyses SET state='working' WHERE goal_id=?1",
                    [id],
                )
                .unwrap();
        }
        {
            let storage = Storage::open(&path).unwrap();
            assert_eq!(storage.goal_analysis_view(id).unwrap().state, "queued");
        }
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn removing_today_focus_does_not_defer_the_goal_and_moves_to_the_next() {
        let mut storage = Storage::open(std::path::Path::new(":memory:")).unwrap();
        let date = chrono::Local::now().date_naive().to_string();
        storage
            .core_create_goal("First goal", "first", &date)
            .unwrap();
        let first = storage.ensure_today_focus().unwrap().unwrap();
        storage
            .core_create_goal("Second goal", "second", &date)
            .unwrap();
        storage.core_today(first, false, &date).unwrap();
        assert_ne!(storage.ensure_today_focus().unwrap(), Some(first));
        assert_eq!(
            storage
                .core_snapshot(None)
                .unwrap()
                .goals
                .iter()
                .find(|g| g.id == first)
                .unwrap()
                .status,
            "open"
        );
        storage.core_plan_day(&date, true).unwrap();
        assert!(storage.ensure_today_focus().unwrap().is_none());
    }
}
