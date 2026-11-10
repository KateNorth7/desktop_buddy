//! Bounded evidence of what already permitted observations showed during a work episode,
//! and concise summaries of episodes (Prompt B of docs/GOAL_UNDERSTANDING_PROMPTS.md).
//! Reviewers consume this and never read the screen themselves.
//!
//! Storage extends the existing review tables instead of adding a parallel store: the
//! measured windows are `review_intervals` and `review_contexts`, the sealed text and
//! image samples are `review_evidence`, and the only new field is the fixed result code a
//! text sample showed (`review_evidence.result_signal`). So this evidence persists across
//! restarts, is deleted with the review period after the daily summary, follows retention
//! and Clear local history, and cascades with nothing it does not own.
//!
//! Limits: at most `MAX_ITEMS` windows per read; excerpts are read on demand, redacted
//! when sealed and bounded to `EXCERPT_CHARS`; episodes split at `EPISODE_GAP` and at
//! `SUMMARY_SPAN`; a summary lists at most `SUMMARY_SOURCES` windows in `SUMMARY_CHARS`.
//! Summaries are computed from the stored evidence when read, so they cannot drift from
//! it or turn a guess into proof: they describe measured windows and observed result
//! codes, never inferred purposes.
// Prompt C's episode review is the consumer; until it lands, only tests read these.
#![cfg_attr(not(test), allow(dead_code))]
use crate::storage::Storage;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

pub(crate) const MAX_ITEMS: usize = 200;
pub(crate) const EXCERPT_CHARS: usize = 600;
pub(crate) const EPISODE_GAP: chrono::Duration = chrono::Duration::seconds(120);
pub(crate) const SUMMARY_SPAN: chrono::Duration = chrono::Duration::hours(2);
pub(crate) const SUMMARY_SOURCES: usize = 12;
pub(crate) const SUMMARY_CHARS: usize = 600;

/// Phrases a meeting shows before joining, and once in the call, in English and Russian.
pub(crate) const PRE_JOIN: &[&str] = &[
    "join now",
    "ask to join",
    "ready to join",
    "присоединиться",
    "готовы присоединиться",
];
pub(crate) const IN_CALL: &[&str] = &[
    "leave call",
    "end call",
    "leave meeting",
    "покинуть звонок",
    "завершить звонок",
    "выйти из встречи",
];
/// Confirmation phrases mail clients show once a message has left.
pub(crate) const MESSAGE_SENT: &[&str] = &[
    "message sent",
    "email sent",
    "your message has been sent",
    "письмо отправлено",
    "сообщение отправлено",
    "ваше письмо отправлено",
];
/// A submitted form's confirmation, and signs that the submission failed.
pub(crate) const FORM_DONE: &[&str] = &[
    "thank you for applying",
    "application submitted",
    "application has been submitted",
    "thanks for submitting",
    "submission received",
    "successfully submitted",
    "your response has been recorded",
    "заявка отправлена",
    "спасибо за отклик",
    "ответ записан",
];
pub(crate) const FORM_ERROR: &[&str] = &[
    "error",
    "required field",
    "please correct",
    "something went wrong",
    "ошибка",
    "обязательное поле",
];
pub(crate) fn says(text: &str, phrases: &[&str]) -> bool {
    let text = text.to_lowercase();
    phrases.iter().any(|p| text.contains(p))
}
/// The fixed result code a text sample shows, if any. Errors win over confirmations.
pub(crate) fn result_signal(text: &str) -> Option<&'static str> {
    if says(text, FORM_DONE) {
        return Some(if says(text, FORM_ERROR) {
            "form_error"
        } else {
            "form_submitted"
        });
    }
    if says(text, MESSAGE_SENT) {
        return Some("message_sent");
    }
    if says(text, IN_CALL) {
        return Some("in_call");
    }
    if says(text, PRE_JOIN) {
        return Some("pre_join");
    }
    None
}

/// One retained window of an episode. `excerpt_ref` resolves through `episode_excerpt`;
/// raw text never travels in this type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct EpisodeEvidence {
    pub at: String,
    pub context_key: String,
    pub app: String,
    pub site: Option<String>,
    pub excerpt_ref: Option<String>,
    /// The newest fixed result code observed in the window, for example "message_sent".
    pub result_signal: Option<String>,
    pub goal_id: Option<i64>,
    pub step_id: Option<String>,
    /// "interface_text" when sealed text exists for the window, else "window_metadata".
    pub provenance: String,
    /// Consent is not persisted across restarts; consumers recheck current consent and
    /// exclusions before any disclosure. Always 0 for stored evidence.
    pub consent_revision: u64,
    /// Measured active seconds of the window within the range.
    #[serde(default)]
    pub seconds: i64,
    /// Minimized window title, as already stored for the daily review.
    #[serde(default)]
    pub title: String,
}

/// A concise description of one episode: measured windows and observed results only.
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

fn db(e: rusqlite::Error) -> String {
    e.to_string()
}
impl Storage {
    /// Windows measured within the range, oldest first, with their newest result code and
    /// the goal they are attributed to. Idle, locked and excluded time was never recorded.
    pub(crate) fn episode_evidence(
        &self,
        range: EpisodeRange,
    ) -> Result<Vec<EpisodeEvidence>, String> {
        let mut q = self
            .connection
            .prepare(
                "SELECT i.period_id,i.context_key,MIN(i.started_at),i.app,i.title,MAX(i.domain),
                    SUM(i.milliseconds)/1000,
                    (SELECT x.goal_id FROM review_intervals x WHERE x.period_id=i.period_id AND x.context_key=i.context_key AND x.status='assigned' AND x.goal_id IS NOT NULL GROUP BY x.goal_id ORDER BY SUM(x.milliseconds) DESC LIMIT 1),
                    (SELECT x.source_step FROM review_intervals x WHERE x.period_id=i.period_id AND x.context_key=i.context_key AND x.source_step<>'' ORDER BY x.id DESC LIMIT 1),
                    EXISTS(SELECT 1 FROM review_evidence e WHERE e.period_id=i.period_id AND e.context_key=i.context_key AND e.kind='text'),
                    (SELECT e.result_signal FROM review_evidence e WHERE e.period_id=i.period_id AND e.context_key=i.context_key AND e.result_signal IS NOT NULL ORDER BY e.id DESC LIMIT 1)
                 FROM review_intervals i
                 WHERE julianday(i.ended_at)>julianday(?1) AND julianday(i.started_at)<julianday(?2)
                 GROUP BY i.period_id,i.context_key ORDER BY MIN(i.started_at) LIMIT ?3",
            )
            .map_err(db)?;
        let rows = q
            .query_map(
                params![
                    range.since.to_rfc3339(),
                    range.until.to_rfc3339(),
                    MAX_ITEMS as i64
                ],
                |r| {
                    let period: i64 = r.get(0)?;
                    let key: String = r.get(1)?;
                    let text: bool = r.get(9)?;
                    Ok(EpisodeEvidence {
                        at: r.get(2)?,
                        excerpt_ref: text.then(|| format!("review_text:{period}:{key}")),
                        context_key: key,
                        app: r.get(3)?,
                        title: r.get(4)?,
                        site: r.get(5)?,
                        seconds: r.get(6)?,
                        goal_id: r.get(7)?,
                        step_id: r.get(8)?,
                        provenance: if text {
                            "interface_text"
                        } else {
                            "window_metadata"
                        }
                        .into(),
                        result_signal: r.get(10)?,
                        consent_revision: 0,
                    })
                },
            )
            .map_err(db)?;
        rows.collect::<Result<_, _>>().map_err(db)
    }
    /// The bounded redacted excerpt behind an `excerpt_ref`. Callers check consent first.
    pub(crate) fn episode_text(&self, excerpt_ref: &str) -> Result<Option<String>, String> {
        let Some(rest) = excerpt_ref.strip_prefix("review_text:") else {
            return Ok(None);
        };
        let Some((period, key)) = rest.split_once(':') else {
            return Ok(None);
        };
        let Ok(period) = period.parse::<i64>() else {
            return Ok(None);
        };
        self.episode_excerpt(period, key, EXCERPT_CHARS)
    }
    /// Episodes within the range: runs of windows without a gap over `EPISODE_GAP`, each
    /// at most `SUMMARY_SPAN` long, oldest first.
    pub(crate) fn episode_summaries(
        &self,
        range: EpisodeRange,
    ) -> Result<Vec<EpisodeSummary>, String> {
        let intervals: Vec<(DateTime<Utc>, DateTime<Utc>, String)> = {
            let mut q = self
                .connection
                .prepare("SELECT started_at,ended_at,context_key FROM review_intervals WHERE julianday(ended_at)>julianday(?1) AND julianday(started_at)<julianday(?2) ORDER BY started_at")
                .map_err(db)?;
            let rows = q
                .query_map(
                    params![range.since.to_rfc3339(), range.until.to_rfc3339()],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(db)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(db)?
                .into_iter()
                .filter_map(|(a, b, k)| {
                    Some((
                        DateTime::parse_from_rfc3339(&a).ok()?.with_timezone(&Utc),
                        DateTime::parse_from_rfc3339(&b).ok()?.with_timezone(&Utc),
                        k,
                    ))
                })
                .collect()
        };
        let mut bounds: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
        for (start, end, _) in &intervals {
            match bounds.last_mut() {
                Some((first, last))
                    if *start - *last <= EPISODE_GAP && *end - *first <= SUMMARY_SPAN =>
                {
                    *last = (*last).max(*end);
                }
                _ => bounds.push((*start, *end)),
            }
        }
        bounds
            .into_iter()
            .map(|(since, until)| {
                let windows = self.episode_evidence(EpisodeRange { since, until })?;
                Ok(summarize(since, until, windows))
            })
            .collect()
    }
}
/// Describes measured windows and observed result codes only, longest windows first.
fn summarize(
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    mut windows: Vec<EpisodeEvidence>,
) -> EpisodeSummary {
    let mut goal_ids: Vec<i64> = windows.iter().filter_map(|w| w.goal_id).collect();
    goal_ids.sort_unstable();
    goal_ids.dedup();
    windows.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.at.cmp(&b.at)));
    windows.truncate(SUMMARY_SOURCES);
    let parts: Vec<String> = windows
        .iter()
        .map(|w| {
            let place = w.site.clone().unwrap_or_else(|| w.app.clone());
            let minutes = (w.seconds + 59) / 60;
            match &w.result_signal {
                Some(signal) => format!("{place} {minutes}m ({signal})"),
                None => format!("{place} {minutes}m"),
            }
        })
        .collect();
    EpisodeSummary {
        started_at: since.to_rfc3339(),
        ended_at: until.to_rfc3339(),
        summary: parts.join("; ").chars().take(SUMMARY_CHARS).collect(),
        sources: windows.into_iter().map(|w| w.context_key).collect(),
        goal_ids,
    }
}

#[cfg(test)]
mod tests;
