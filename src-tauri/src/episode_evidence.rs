//! Shared contract, owned by Prompt B (docs/GOAL_UNDERSTANDING_PROMPTS.md): bounded
//! evidence of what already permitted observations showed during a work episode, and
//! rolling summaries of long episodes. Reviewers consume this and never read the screen
//! themselves.
//!
//! Storage is Prompt B's decision: extend the existing encrypted `review_evidence`,
//! `review_contexts` and `review_intervals` where they fit instead of a parallel store.
//! Until then the reads return nothing, which every consumer must treat as "unknown".
#![allow(dead_code)] // Contract: Prompt B implements storage, Prompt C the consumers.
use crate::storage::Storage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One retained observation. `excerpt_ref` points at a bounded redacted excerpt or at
/// existing encrypted review evidence; raw text never travels in this type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct EpisodeEvidence {
    pub at: String,
    pub context_key: String,
    pub app: String,
    pub site: Option<String>,
    pub excerpt_ref: Option<String>,
    /// A fixed result code when one was observed, for example "message_sent".
    pub result_signal: Option<String>,
    pub goal_id: Option<i64>,
    pub step_id: Option<String>,
    /// Fixed source code, for example "interface_text" or "window_metadata".
    pub provenance: String,
    pub consent_revision: u64,
}

/// A concise summary of a long episode. It references its sources and never turns a
/// guess into proof.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct EpisodeSummary {
    pub started_at: String,
    pub ended_at: String,
    pub summary: String,
    /// Context keys of the evidence the summary rests on.
    pub sources: Vec<String>,
    pub goal_ids: Vec<i64>,
}

/// The wall-clock bounds of one work episode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EpisodeRange {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
}

impl Storage {
    /// Evidence recorded within the range, oldest first. Empty until Prompt B lands.
    pub(crate) fn episode_evidence(
        &self,
        _range: EpisodeRange,
    ) -> Result<Vec<EpisodeEvidence>, String> {
        Ok(Vec::new())
    }
    /// Summaries overlapping the range, oldest first. Empty until Prompt B lands.
    pub(crate) fn episode_summaries(
        &self,
        _range: EpisodeRange,
    ) -> Result<Vec<EpisodeSummary>, String> {
        Ok(Vec::new())
    }
}
