//! The one path through which every analyser (fast signals, episode-end offers, episode
//! review) turns a finding into an action (Prompt C of docs/GOAL_UNDERSTANDING_PROMPTS.md).
//! It checks goal and step identity, the current plan revision, consent, deduplication and
//! cooldowns before anything is queued or changed, and it only uses the existing cards
//! (goal and step completion, new goal, new step, the automatic-completion receipt) and the
//! existing mutation methods (automatic completion with Undo, retroactive attribution).
//! The user's earlier answers stay effective: a dismissed card id is never shown again,
//! and an undone automatic completion is never repeated for the same plan.
use crate::{
    commands::Inner,
    companion::engine::{Intervention, Kind},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecisionKind {
    /// Ask the user to confirm a goal or step is done (existing confirmation card).
    ProposeComplete,
    /// Complete with Undo; only when the setting allows it and the caller holds evidence
    /// connected to the goal's target. Otherwise it becomes a proposal.
    AutoComplete,
    /// Offer a new goal; always needs the user's acceptance.
    ProposeGoal,
    /// Offer a new step; always needs the user's acceptance.
    ProposeStep,
    /// Attribute pending measured time to a goal, retroactively and without double counting.
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
    /// Evidence references: `review:<period>:<context key>` for episode windows.
    pub evidence: Vec<String>,
    /// A short factual reason a card may show, in English.
    pub reason: String,
    pub confidence: f64,
    /// Same key, same finding: it is applied at most once.
    pub dedupe_key: String,
    /// Fixed analyser code: "fast_signal", "episode_time" or "episode_review".
    pub source: String,
    /// The proposed goal or step text, for ProposeGoal and ProposeStep.
    #[serde(default)]
    pub text: Option<String>,
    /// Where the result was observed, recorded with an automatic completion.
    #[serde(default)]
    pub place: Option<String>,
    /// The goal's plan revision the analyser saw; a finding about an older plan is dropped.
    #[serde(default)]
    pub plan_revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    Queued,
    Changed,
    Deduplicated,
    Rejected(&'static str),
    Skipped,
}

/// Session state: findings already applied and per-goal cooldowns.
#[derive(Default)]
pub struct Runtime {
    applied: HashMap<String, Instant>,
    cooldown: HashMap<(Option<i64>, Option<String>, &'static str), Instant>,
}

/// A goal or step is offered for completion at most this often, whichever analyser asks.
const COMPLETE_COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// A new goal or step is offered at most this often per text.
const PROPOSE_COOLDOWN: Duration = Duration::from_secs(4 * 60 * 60);
/// Remembered findings are forgotten after this long.
const MEMORY: Duration = Duration::from_secs(24 * 60 * 60);
/// Card lifetimes.
const PROPOSAL_TTL: i64 = 3600;
const RECEIPT_TTL: i64 = 86400;

fn note(inner: &Inner, outcome: &str, goal: Option<i64>, confidence: f64) {
    let _ = inner.storage.note_attempt(
        crate::tracking_attempts::OUTCOME,
        outcome,
        goal,
        Some(confidence),
    );
}
fn consent(inner: &Inner) -> bool {
    inner.companion.view.preferences.screen_task_detection
        && inner
            .storage
            .ai_preferences()
            .is_ok_and(|p| p.enabled && p.share_goal_context)
}
fn bounded(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

/// Routes one finding through the shared checks and applies it with the existing tools.
/// `now` is the monotonic time of the finding, so cooldowns can be driven in replays.
pub(crate) fn submit_at(
    inner: &mut Inner,
    decision: Decision,
    now: Instant,
) -> Result<Applied, String> {
    inner
        .decisions
        .applied
        .retain(|_, at| now.saturating_duration_since(*at) < MEMORY);
    if decision.kind == DecisionKind::None {
        return Ok(Applied::Skipped);
    }
    if !decision.confidence.is_finite() || !(0.0..=1.0).contains(&decision.confidence) {
        return Ok(Applied::Rejected("invalid_confidence"));
    }
    if inner.decisions.applied.contains_key(&decision.dedupe_key) {
        note(
            inner,
            "proposal_deduplicated",
            decision.goal_id,
            decision.confidence,
        );
        return Ok(Applied::Deduplicated);
    }
    if !consent(inner) && decision.kind != DecisionKind::Attribute {
        return Ok(Applied::Rejected("consent"));
    }
    // Identity and freshness: the goal is open, the step exists and is not done.
    let snapshot = inner.storage.core_work_snapshot()?;
    let goal = decision.goal_id.and_then(|id| {
        snapshot
            .goals
            .iter()
            .find(|g| g.id == id && g.status == "open")
    });
    if decision.kind != DecisionKind::ProposeGoal && goal.is_none() {
        note(
            inner,
            "evidence_expired",
            decision.goal_id,
            decision.confidence,
        );
        return Ok(Applied::Rejected("goal_changed"));
    }
    if let (Some(goal), Some(seen)) = (goal, decision.plan_revision) {
        if decision.kind != DecisionKind::Attribute && goal.plan.revision != seen {
            note(
                inner,
                "evidence_expired",
                decision.goal_id,
                decision.confidence,
            );
            return Ok(Applied::Rejected("plan_changed"));
        }
    }
    if let (Some(goal), Some(step)) = (goal, decision.step_id.as_ref()) {
        if !goal.plan.steps.iter().any(|s| &s.id == step && !s.done) {
            note(
                inner,
                "evidence_expired",
                decision.goal_id,
                decision.confidence,
            );
            return Ok(Applied::Rejected("step_changed"));
        }
    }
    let wall = chrono::Utc::now().timestamp();
    let result = match decision.kind {
        DecisionKind::Attribute => attribute(inner, &decision)?,
        DecisionKind::ProposeComplete | DecisionKind::AutoComplete => {
            let goal = goal.expect("checked above");
            let key = (decision.goal_id, decision.step_id.clone(), "complete");
            if inner
                .decisions
                .cooldown
                .get(&key)
                .is_some_and(|at| now.saturating_duration_since(*at) < COMPLETE_COOLDOWN)
            {
                note(
                    inner,
                    "proposal_deduplicated",
                    decision.goal_id,
                    decision.confidence,
                );
                return Ok(Applied::Deduplicated);
            }
            // The goal card can only close a goal without unfinished steps.
            if decision.step_id.is_none() && goal.plan.steps.iter().any(|s| !s.done) {
                return Ok(Applied::Rejected("open_steps"));
            }
            let title = decision
                .step_id
                .as_ref()
                .and_then(|id| goal.plan.steps.iter().find(|s| &s.id == id))
                .map(|s| s.text.clone())
                .unwrap_or_else(|| goal.title.clone());
            let card = Intervention {
                id: format!(
                    "decision-{}",
                    crate::companion::engine::fingerprint(&decision.dedupe_key)
                ),
                kind: if decision.step_id.is_some() {
                    Kind::Completion
                } else {
                    Kind::GoalCompletion
                },
                text: if decision.reason.trim().is_empty() {
                    title.clone()
                } else {
                    format!("{} “{title}”", bounded(&decision.reason, 180))
                },
                confidence: decision.confidence,
                goal_id: decision.goal_id,
                plan_revision: Some(goal.plan.revision),
                step_id: decision.step_id.clone(),
                expires_at: wall + PROPOSAL_TTL,
            };
            inner.decisions.cooldown.insert(key, now);
            let automatic = decision.kind == DecisionKind::AutoComplete;
            if automatic && !inner.companion.view.preferences.automatic_completion {
                note(
                    inner,
                    "automatic_completion_disabled",
                    decision.goal_id,
                    decision.confidence,
                );
                crate::companion::queue_prompt(inner, card, wall);
                Applied::Queued
            } else if automatic {
                complete(inner, &decision, card, &title, wall)?
            } else {
                note(
                    inner,
                    if matches!(
                        decision.source.as_str(),
                        "episode_time" | "semantic_observation"
                    ) {
                        "completion_proposed"
                    } else {
                        "outcome_probable"
                    },
                    decision.goal_id,
                    decision.confidence,
                );
                crate::companion::queue_prompt(inner, card, wall);
                Applied::Queued
            }
        }
        DecisionKind::ProposeGoal | DecisionKind::ProposeStep => {
            let text = bounded(decision.text.as_deref().unwrap_or_default(), 200);
            if text.is_empty() {
                return Ok(Applied::Rejected("empty_text"));
            }
            let fingerprint = crate::companion::engine::fingerprint(&text.to_lowercase());
            let duplicate = if decision.kind == DecisionKind::ProposeGoal {
                snapshot.goals.iter().any(|g| {
                    g.status == "open"
                        && crate::companion::engine::fingerprint(&g.title.to_lowercase())
                            == fingerprint
                })
            } else {
                goal.is_some_and(|g| {
                    g.plan.steps.iter().any(|s| {
                        crate::companion::engine::fingerprint(&s.text.to_lowercase()) == fingerprint
                    })
                })
            };
            if duplicate {
                return Ok(Applied::Rejected("already_listed"));
            }
            let key = (decision.goal_id, Some(fingerprint), "propose");
            if inner
                .decisions
                .cooldown
                .get(&key)
                .is_some_and(|at| now.saturating_duration_since(*at) < PROPOSE_COOLDOWN)
            {
                note(
                    inner,
                    "proposal_deduplicated",
                    decision.goal_id,
                    decision.confidence,
                );
                return Ok(Applied::Deduplicated);
            }
            inner.decisions.cooldown.insert(key, now);
            let card = Intervention {
                id: format!(
                    "decision-{}",
                    crate::companion::engine::fingerprint(&decision.dedupe_key)
                ),
                kind: if decision.kind == DecisionKind::ProposeGoal {
                    Kind::NewGoal
                } else {
                    Kind::NewTask
                },
                text,
                confidence: decision.confidence,
                goal_id: if decision.kind == DecisionKind::ProposeGoal {
                    None
                } else {
                    decision.goal_id
                },
                plan_revision: if decision.kind == DecisionKind::ProposeGoal {
                    None
                } else {
                    goal.map(|g| g.plan.revision)
                },
                step_id: None,
                expires_at: wall + PROPOSAL_TTL,
            };
            note(
                inner,
                "proposal_queued",
                decision.goal_id,
                decision.confidence,
            );
            crate::companion::queue_prompt(inner, card, wall);
            Applied::Queued
        }
        DecisionKind::None => Applied::Skipped,
    };
    if matches!(result, Applied::Queued | Applied::Changed) {
        inner.decisions.applied.insert(decision.dedupe_key, now);
    }
    Ok(result)
}
/// Records a completion applied outside this path (the verified screen observation), so
/// no other analyser offers the same goal or step again within the cooldown.
pub(crate) fn completed_elsewhere(
    inner: &mut Inner,
    goal_id: Option<i64>,
    step_id: Option<String>,
    now: Instant,
) {
    inner
        .decisions
        .cooldown
        .insert((goal_id, step_id, "complete"), now);
}
/// Automatic completion through the existing receipt path, which keeps a durable Undo and
/// refuses a plan the user already undid.
fn complete(
    inner: &mut Inner,
    decision: &Decision,
    card: Intervention,
    title: &str,
    wall: i64,
) -> Result<Applied, String> {
    let prompt = Intervention {
        confidence: decision
            .confidence
            .max(crate::work_journal::MIN_VERIFIED_CONFIDENCE),
        ..card
    };
    let place = decision
        .place
        .clone()
        .unwrap_or_else(|| decision.source.clone());
    match inner
        .storage
        .automatically_complete(&prompt, &place, &decision.dedupe_key)
    {
        Ok(event) => {
            note(
                inner,
                "completion_applied",
                decision.goal_id,
                decision.confidence,
            );
            crate::companion::queue_prompt(
                inner,
                Intervention {
                    id: format!("work-event-{event}"),
                    kind: Kind::AutomaticCompletion,
                    text: format!("Closed “{title}”."),
                    confidence: 1.0,
                    goal_id: decision.goal_id,
                    plan_revision: None,
                    step_id: decision.step_id.clone(),
                    expires_at: wall + RECEIPT_TTL,
                },
                wall,
            );
            Ok(Applied::Changed)
        }
        Err(error)
            if error == "Automatic completion was undone for this plan"
                || error == "This receipt was already reviewed" =>
        {
            Ok(Applied::Rejected("undone_or_reviewed"))
        }
        Err(error) => Err(error),
    }
}
/// Pending windows named by the evidence move to the goal; assigned and manual time never moves.
fn attribute(inner: &mut Inner, decision: &Decision) -> Result<Applied, String> {
    let Some(goal) = decision.goal_id else {
        return Ok(Applied::Rejected("no_goal"));
    };
    let mut moved = 0;
    for reference in &decision.evidence {
        let Some(rest) = reference.strip_prefix("review:") else {
            continue;
        };
        let Some((period, key)) = rest.split_once(':') else {
            continue;
        };
        let Ok(period) = period.parse::<i64>() else {
            continue;
        };
        moved += inner.storage.assign_pending_context(
            period,
            key,
            goal,
            &bounded(&decision.reason, 200),
            decision.confidence,
        )?;
    }
    Ok(if moved > 0 {
        Applied::Changed
    } else {
        Applied::Skipped
    })
}

#[cfg(test)]
mod tests;
