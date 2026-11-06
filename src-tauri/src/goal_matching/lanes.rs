//! Lane state for matching: which goals are current, which were worked on recently and
//! which have been parked for long. Parking is automatic, so recency comes from evidence
//! the app already records instead of a manual choice.
use super::*;
use crate::storage::Storage;
use chrono::{DateTime, Utc};

/// A lane worked on within this window still competes on equal terms.
const RECENT_HOURS: i64 = 24;
/// A parked lane idle for this long leaves the matcher's candidate set unless its own words match.
pub(super) const STALE_PARKED_DAYS: i64 = 14;

pub(crate) struct Lane {
    pub id: i64,
    pub active: bool,
    pub in_today: bool,
    pub last_active_at: DateTime<Utc>,
}
impl Lane {
    pub(super) fn idle_days(&self, now: DateTime<Utc>) -> i64 {
        (now - self.last_active_at).num_days().max(0)
    }
    pub(crate) fn recent(&self, now: DateTime<Utc>) -> bool {
        self.active || (now - self.last_active_at).num_hours() < RECENT_HOURS
    }
    pub(super) fn stale(&self, now: DateTime<Utc>) -> bool {
        !self.active && !self.in_today && self.idle_days(now) >= STALE_PARKED_DAYS
    }
    pub(super) fn json(&self, now: DateTime<Utc>) -> Value {
        json!({
            "active": self.active,
            "in_today": self.in_today,
            "last_worked_days_ago": self.idle_days(now),
        })
    }
}
impl Storage {
    /// Last activity is the latest of creation, a counted interval, a goal or step event and a
    /// saved checkpoint. Lane switches record no time of their own.
    pub(crate) fn lane_state(&self, today: &[i64]) -> Result<Vec<Lane>, String> {
        let mut q = self.connection.prepare(
            "SELECT g.id,g.status='active',MAX(g.created_at,
                COALESCE((SELECT MAX(u.ended_at) FROM usage_intervals u WHERE u.goal_id=g.id AND u.match_confidence>=0.7),''),
                COALESCE((SELECT MAX(e.created_at) FROM core_events e WHERE e.goal_id=g.id),''),
                COALESCE((SELECT w.updated_at FROM work_contexts w WHERE w.goal_id=g.id),''))
             FROM goals g JOIN core_goals c ON c.goal_id=g.id WHERE c.state='open'",
        ).map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|(id, active, last)| Lane {
                id,
                active,
                in_today: today.contains(&id),
                last_active_at: DateTime::parse_from_rfc3339(&last)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
            .collect())
    }
}
/// Among lanes that tie on local evidence, the single recently active one wins.
/// Several recent lanes, or none, stay ambiguous exactly as before.
pub(super) fn recent_tie_break(
    storage: &Storage,
    today: &[i64],
    tied: &[i64],
    now: DateTime<Utc>,
) -> Result<Option<i64>, String> {
    if tied.len() < 2 {
        return Ok(None);
    }
    let lanes = storage.lane_state(today)?;
    let recent: Vec<i64> = tied
        .iter()
        .copied()
        .filter(|id| lanes.iter().any(|l| l.id == *id && l.recent(now)))
        .collect();
    Ok((recent.len() == 1).then(|| recent[0]))
}
