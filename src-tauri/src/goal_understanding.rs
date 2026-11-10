//! Shared contract, owned by Prompt A (docs/GOAL_UNDERSTANDING_PROMPTS.md): what sumi
//! understands a goal to mean and which results would achieve it. The user writes the
//! goal once in any words; this interpretation is internal, never shown or required,
//! and never replaces the goal's title, steps or the user's own done criterion.
//!
//! Lifecycle: one row per goal, replaced when the goal's plan revision changes; deleted
//! with the goal (cascade) and by Clear local history. It holds model output derived
//! from permitted context, so it follows the same consent as sharing goal context.
#![allow(dead_code)] // Contract: Prompt A implements the producer, Prompt C the consumers.
use crate::storage::Storage;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Goal shapes with a local fast path. They help recognise results quickly; they never
/// limit which goals can be understood.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FastKind {
    JoinCall,
    VisitSite,
    SendEmail,
    SubmitForm,
}

/// The validated interpretation of one goal at one plan revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct GoalUnderstanding {
    pub goal_id: i64,
    pub plan_revision: u64,
    /// One sentence: what the user means.
    pub meaning: String,
    /// Plausible successful results, as hypotheses rather than new requirements.
    pub outcomes: Vec<String>,
    /// People, organisations or projects the goal is about, only when supported.
    pub entities: Vec<String>,
    /// Concrete targets a result must connect to: a recipient, meeting, site or form.
    pub targets: Vec<String>,
    pub kind: Option<FastKind>,
    /// 0 is certain, 1 is a guess.
    pub uncertainty: f64,
    /// Fixed source codes, for example "goal_text", "creation_source", "episode".
    pub provenance: Vec<String>,
    pub updated_at: String,
}

fn db(e: rusqlite::Error) -> String {
    e.to_string()
}
impl Storage {
    pub(crate) fn initialize_goal_understanding(&self) -> Result<(), String> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS goal_understandings(goal_id INTEGER PRIMARY KEY REFERENCES goals(id) ON DELETE CASCADE,plan_revision INTEGER NOT NULL,payload TEXT NOT NULL,updated_at TEXT NOT NULL);",
            )
            .map_err(db)
    }
    /// The stored interpretation, only while it matches the goal's current plan revision.
    pub(crate) fn goal_understanding(
        &self,
        goal: i64,
    ) -> Result<Option<GoalUnderstanding>, String> {
        let revision = self.goal_plan(goal)?.revision;
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM goal_understandings WHERE goal_id=?1 AND plan_revision=?2",
                params![goal, revision as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(|_| "Invalid goal understanding".to_string()))
            .transpose()
    }
    /// Replaces the interpretation of a goal. Callers validate it first.
    pub(crate) fn save_goal_understanding(
        &self,
        understanding: &GoalUnderstanding,
    ) -> Result<(), String> {
        let payload = serde_json::to_string(understanding).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO goal_understandings(goal_id,plan_revision,payload,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(goal_id) DO UPDATE SET plan_revision=excluded.plan_revision,payload=excluded.payload,updated_at=excluded.updated_at",
                params![
                    understanding.goal_id,
                    understanding.plan_revision as i64,
                    payload,
                    understanding.updated_at
                ],
            )
            .map_err(db)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::GoalDraft;
    #[test]
    fn an_understanding_is_read_only_at_its_revision_and_follows_the_goal() {
        let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
        s.core_add_goals(
            vec![GoalDraft {
                title: "Sort out the interview".into(),
                area: "Work".into(),
                steps: vec![],
            }],
            "contract",
        )
        .unwrap();
        let id = s.core_snapshot(None).unwrap().goals[0].id;
        assert_eq!(s.goal_understanding(id).unwrap(), None);
        let revision = s.goal_plan(id).unwrap().revision;
        let understanding = GoalUnderstanding {
            goal_id: id,
            plan_revision: revision,
            meaning: "Resolve the pending interview request".into(),
            outcomes: vec!["Availability sent".into(), "Decline sent".into()],
            entities: vec![],
            targets: vec![],
            kind: None,
            uncertainty: 0.4,
            provenance: vec!["goal_text".into()],
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        s.save_goal_understanding(&understanding).unwrap();
        assert_eq!(
            s.goal_understanding(id).unwrap(),
            Some(understanding.clone())
        );
        // A stale revision is never returned.
        s.save_goal_understanding(&GoalUnderstanding {
            plan_revision: revision + 7,
            ..understanding
        })
        .unwrap();
        assert_eq!(s.goal_understanding(id).unwrap(), None);
        // Clear local history removes it.
        s.purge_history(None).unwrap();
        let rows: i64 = s
            .connection
            .query_row("SELECT COUNT(*) FROM goal_understandings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }
}
