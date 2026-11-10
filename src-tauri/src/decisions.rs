//! Shared contract, owned by Prompt C (docs/GOAL_UNDERSTANDING_PROMPTS.md): the one path
//! through which every analyser (fast signals, semantic observation, episode review,
//! unlisted work) turns a finding into an action. It enforces identity, revisions,
//! consent, freshness, deduplication, rejection, cooldown and Undo before anything is
//! queued or changed, and it uses the existing cards and mutation methods.
//!
//! Until Prompt C lands nothing calls `submit`, and the stub changes nothing.
#![allow(dead_code)] // Contract: Prompt C implements this and moves the callers onto it.
use crate::commands::Inner;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecisionKind {
    /// Ask the user to confirm a goal or step is done (existing confirmation card).
    ProposeComplete,
    /// Complete with Undo; only when the setting allows it and verification passes.
    AutoComplete,
    /// Offer a new goal; always needs the user's acceptance.
    ProposeGoal,
    /// Offer a new step; always needs the user's acceptance.
    ProposeStep,
    /// Attribute measured time to a goal, retroactively and without double counting.
    Attribute,
    /// Record that the evidence supported no action.
    None,
}

/// One finding from any analyser.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Decision {
    pub goal_id: Option<i64>,
    pub step_id: Option<String>,
    pub kind: DecisionKind,
    /// Context keys of the supporting evidence (see `episode_evidence`).
    pub evidence: Vec<String>,
    /// A short factual reason a card may show, in English.
    pub reason: String,
    pub confidence: f64,
    /// Same key, same finding: it is applied at most once.
    pub dedupe_key: String,
    /// Fixed analyser code, for example "fast_signal" or "episode_review".
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    Queued,
    Changed,
    Deduplicated,
    Rejected(&'static str),
    Skipped,
}

/// Routes a finding through the shared checks. The contract stub changes nothing.
pub(crate) fn submit(_inner: &mut Inner, _decision: Decision) -> Result<Applied, String> {
    Ok(Applied::Skipped)
}
