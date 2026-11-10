//! One runtime queue for mascot messages. Observation consent is separate from delivery.
use super::{
    deadlines,
    engine::{Intervention, Kind},
    schedule, Inner,
};

#[derive(Default)]
pub(super) struct Queue {
    items: Vec<Pending>,
    current_consent: Option<u64>,
}
struct Pending {
    prompt: Intervention,
    queued_at: i64,
    delivered: bool,
    consent: u64,
}
impl Queue {
    pub(super) fn clear(&mut self) {
        self.items.clear();
        self.current_consent = None;
    }
}
pub(super) fn priority(p: &Intervention) -> u8 {
    match p.kind {
        Kind::Deadline if p.id.ends_with(":now") => 0,
        Kind::OnboardingCompletion
        | Kind::Completion
        | Kind::GoalCompletion
        | Kind::AutomaticCompletion
        | Kind::UnplannedCompleted => 1,
        Kind::EndOfDay => 2,
        Kind::Deadline => 3,
        Kind::NewTask | Kind::NewGoal | Kind::UnplannedWork | Kind::GoalMatch => 4,
        Kind::Movement => 5,
        _ => 6,
    }
}
fn same_question(a: &Intervention, b: &Intervention) -> bool {
    a.id == b.id
        || (a.kind == b.kind
            && match a.kind {
                Kind::Movement | Kind::NoGoals | Kind::EndOfDay | Kind::GoalMatch => true,
                Kind::Deadline => a.goal_id == b.goal_id,
                Kind::Completion => a.goal_id == b.goal_id && a.step_id == b.step_id,
                Kind::GoalCompletion => a.goal_id == b.goal_id,
                Kind::NewTask | Kind::NewGoal => {
                    a.goal_id == b.goal_id
                        && super::fingerprint(&a.text) == super::fingerprint(&b.text)
                }
                Kind::UnplannedWork | Kind::UnplannedCompleted => true,
                _ => false,
            })
}
/// A receipt for a change already made (an automatic completion with Undo) must reach the
/// user; quiet mode postpones it instead of discarding it. Questions are still dropped.
fn survives_quiet(p: &Intervention) -> bool {
    p.kind == Kind::AutomaticCompletion
}
pub(super) fn enqueue(inner: &mut Inner, prompt: Intervention, now: i64) {
    if (super::quiet_mode(inner) && !survives_quiet(&prompt))
        || prompt.expires_at <= now
        || matches!(prompt.kind, Kind::Midday | Kind::Research)
        || inner.companion.memory.seen.contains(&prompt.id)
        || inner
            .companion
            .view
            .intervention
            .as_ref()
            .is_some_and(|p| same_question(p, &prompt) && valid(inner, p, now))
    {
        return;
    }
    let queue = &mut inner.companion.queue;
    if let Some(old) = queue
        .items
        .iter_mut()
        .find(|p| same_question(&p.prompt, &prompt))
    {
        if old.consent != inner.last_consent_change
            || (old.prompt.id != prompt.id
                && matches!(
                    prompt.kind,
                    Kind::Deadline
                        | Kind::GoalMatch
                        | Kind::UnplannedWork
                        | Kind::UnplannedCompleted
                ))
        {
            *old = Pending {
                prompt,
                queued_at: now,
                delivered: false,
                consent: inner.last_consent_change,
            };
            return;
        }
        // Refresh evidence without extending the lifetime or postponing the courtesy delay.
        let expires = old.prompt.expires_at.min(prompt.expires_at);
        old.prompt = Intervention {
            id: old.prompt.id.clone(),
            expires_at: expires,
            ..prompt
        };
        return;
    }
    queue.items.push(Pending {
        prompt,
        queued_at: now,
        delivered: false,
        consent: inner.last_consent_change,
    });
}
fn observed_question(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::NewTask
            | Kind::NewGoal
            | Kind::Completion
            | Kind::GoalCompletion
            | Kind::UnplannedWork
            | Kind::UnplannedCompleted
            | Kind::GoalMatch
    )
}
fn valid(inner: &Inner, p: &Intervention, now: i64) -> bool {
    if p.expires_at <= now {
        return false;
    }
    match p.kind {
        Kind::OnboardingCompletion => inner.storage.valid_onboarding_prompt(p, now),
        Kind::Deadline => inner.companion.view.preferences.deadline_reminders
            && deadlines::validate(&inner.storage, p, "", now).is_ok(),
        Kind::NoGoals => !inner.storage.onboarding_active().unwrap_or(false) && inner.companion.view.preferences.daily_checkins
            && p.id.starts_with(&format!("schedule:{}:planning", chrono::DateTime::from_timestamp(now,0).unwrap().with_timezone(&chrono::Local).date_naive())),
        Kind::EndOfDay => inner.companion.view.preferences.daily_checkins && schedule::allowed(inner, p.kind, now),
        Kind::Movement => inner.companion.view.preferences.movement_reminder
            && schedule::preferences(&inner.storage).is_ok_and(|p| p.movement_reminders),
        Kind::UnplannedWork | Kind::UnplannedCompleted => crate::unplanned_work::queued_current(inner, &p.id),
        Kind::GoalMatch => p.goal_id.is_some_and(|id| crate::goal_matching::suggestions::queued_current(inner, id)),
        Kind::AutomaticCompletion => p.id.strip_prefix("work-event-").and_then(|s| s.parse::<i64>().ok()).is_some_and(|id| {
            inner.storage.connection.query_row("SELECT EXISTS(SELECT 1 FROM work_events WHERE id=?1 AND status='completed')", [id], |r| r.get::<_,bool>(0)).unwrap_or(false)
        }),
        Kind::NewGoal => inner.companion.view.preferences.screen_task_detection
            && inner.storage.ai_preferences().is_ok_and(|p| p.enabled && p.share_goal_context)
            && p.goal_id.is_none() && p.plan_revision.is_none()
            && inner.storage.core_work_snapshot().is_ok_and(|s| !s.goals.iter().any(|g| g.status=="open" && super::fingerprint(&g.title)==super::fingerprint(&p.text))),
        Kind::NewTask | Kind::Completion | Kind::GoalCompletion => inner.companion.view.preferences.screen_task_detection
            && inner.storage.ai_preferences().is_ok_and(|p| p.enabled && p.share_goal_context)
            && p.goal_id.is_some_and(|id| inner.storage.core_work_snapshot().is_ok_and(|s|s.goals.iter().any(|g|g.id==id&&g.status=="open")))
            && p.goal_id.is_some_and(|id| inner.storage.goal_plan(id).is_ok_and(|plan| Some(plan.revision) == p.plan_revision)),
        _ => false,
    }
}
fn ready(inner: &Inner, pending: &Pending, now: i64) -> bool {
    // Activity questions get a bounded courtesy delay. Typing can never starve delivery.
    !matches!(
        pending.prompt.kind,
        Kind::NewTask
            | Kind::NewGoal
            | Kind::UnplannedWork
            | Kind::UnplannedCompleted
            | Kind::GoalMatch
    ) || pending.delivered
        || now.saturating_sub(pending.queued_at) >= 10
        || inner
            .buddy
            .foreground
            .as_ref()
            .is_none_or(|a| a.idle_seconds >= 5)
}
fn reserve(inner: &mut Inner, prompt: &Intervention, now: i64) -> Result<bool, String> {
    let before = inner.companion.memory.clone();
    inner
        .storage
        .connection
        .execute_batch("SAVEPOINT mascot_delivery")
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let reserved = if prompt.kind == Kind::OnboardingCompletion {
            // Durable milestone responses own deduplication, independent of AI cooldowns.
            true
        } else if prompt.kind == Kind::Deadline {
            deadlines::reserve(&inner.storage, &mut inner.companion.memory, prompt, "", now)?
        } else if matches!(
            prompt.kind,
            Kind::Completion | Kind::GoalCompletion | Kind::AutomaticCompletion
        ) {
            inner.companion.memory.reserve_progress(prompt)
        } else {
            inner.companion.memory.reserve(prompt, now)
        };
        if reserved {
            inner
                .storage
                .write_setting("companion_memory", &inner.companion.memory)?;
        }
        Ok(reserved)
    })();
    let finish = inner
        .storage
        .connection
        .execute_batch(if result.is_ok() {
            "RELEASE mascot_delivery"
        } else {
            "ROLLBACK TO mascot_delivery; RELEASE mascot_delivery"
        })
        .map_err(|e| e.to_string());
    if result.is_err() || finish.is_err() {
        inner.companion.memory = before;
        if finish.is_err() {
            let _ = inner
                .storage
                .connection
                .execute_batch("ROLLBACK TO mascot_delivery; RELEASE mascot_delivery");
        }
    }
    finish?;
    result
}
pub(super) fn deliver(inner: &mut Inner, now: i64) -> Result<Option<Intervention>, String> {
    let mut queue = std::mem::take(&mut inner.companion.queue);
    queue.items.retain(|p| {
        valid(inner, &p.prompt, now)
            && (!observed_question(p.prompt.kind) || p.consent == inner.last_consent_change)
    });
    if inner.companion.view.intervention.as_ref().is_some_and(|p| {
        !valid(inner, p, now)
            || (observed_question(p.kind)
                && queue
                    .current_consent
                    .is_some_and(|c| c != inner.last_consent_change))
    }) {
        inner.companion.view.intervention = None;
    }
    if super::quiet_mode(inner) {
        queue.items.retain(|p| survives_quiet(&p.prompt));
        // A receipt on screen when quiet mode starts is shown again afterwards.
        if let Some(prompt) = inner
            .companion
            .view
            .intervention
            .take()
            .filter(survives_quiet)
        {
            if !queue.items.iter().any(|p| p.prompt.id == prompt.id) {
                queue.items.push(Pending {
                    prompt,
                    // Already counted as shown; it returns without a second reservation.
                    delivered: true,
                    consent: inner.last_consent_change,
                    queued_at: now,
                });
            }
        }
        if queue.items.is_empty() {
            queue.current_consent = None;
        }
        inner.companion.queue = queue;
        return Ok(None);
    }
    let blocked = super::suppressed(inner);
    if blocked {
        if let Some(prompt) = inner
            .companion
            .view
            .intervention
            .take()
            .filter(|p| p.kind != Kind::NoGoals)
        {
            queue.items.push(Pending {
                prompt,
                delivered: true,
                consent: queue.current_consent.unwrap_or(inner.last_consent_change),
                queued_at: now,
            });
        }
        inner.companion.queue = queue;
        return Ok(None);
    }
    let next = queue
        .items
        .iter()
        .enumerate()
        .filter(|(_, p)| ready(inner, p, now))
        .min_by_key(|(_, p)| (priority(&p.prompt), p.queued_at))
        .map(|(i, _)| i);
    let Some(index) = next else {
        inner.companion.queue = queue;
        return Ok(None);
    };
    if inner
        .companion
        .view
        .intervention
        .as_ref()
        .is_some_and(|p| priority(p) <= priority(&queue.items[index].prompt))
    {
        inner.companion.queue = queue;
        return Ok(None);
    }
    let pending = queue.items.remove(index);
    let mut prompt = pending.prompt.clone();
    if prompt.kind == Kind::EndOfDay {
        let date = chrono::DateTime::from_timestamp(now, 0)
            .unwrap()
            .with_timezone(&chrono::Local)
            .date_naive();
        match schedule::summary(&inner.storage, &[date]) {
            Ok(text) => prompt.text = text,
            Err(error) => {
                queue.items.push(pending);
                inner.companion.queue = queue;
                return Err(error);
            }
        }
    }
    if prompt.kind == Kind::Deadline {
        match deadlines::refresh(&inner.storage, &prompt, now) {
            Ok(text) => prompt.text = text,
            Err(error) => {
                queue.items.push(pending);
                inner.companion.queue = queue;
                return Err(error);
            }
        }
    }
    if !pending.delivered {
        match reserve(inner, &prompt, now) {
            Ok(true) => {}
            Ok(false) => {
                inner.companion.queue = queue;
                return Ok(None);
            }
            Err(error) => {
                queue.items.push(pending);
                inner.companion.queue = queue;
                return Err(error);
            }
        }
        if prompt.kind == Kind::Movement {
            inner.companion.work_clock.restart();
        }
    }
    if let Some(previous) = inner
        .companion
        .view
        .intervention
        .replace(prompt.clone())
        .filter(|p| p.kind != Kind::NoGoals)
    {
        // Keep a displaced question. Already-shown planning never resurfaces without explicit Not now.
        queue.items.push(Pending {
            prompt: previous,
            delivered: true,
            consent: queue.current_consent.unwrap_or(inner.last_consent_change),
            queued_at: now,
        });
    }
    queue.current_consent = Some(pending.consent);
    inner.companion.view.state = "attention".into();
    inner.companion.queue = queue;
    Ok(Some(prompt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::AppState, models::ActivitySnapshot, storage::Storage};
    use chrono::{Local, TimeZone};
    #[test]
    fn setup_milestones_deliver_with_observation_off_and_respect_quiet_mode() {
        let state = state();
        let mut inner = state.inner.lock().unwrap();
        inner.storage.start_onboarding_goal().unwrap();
        inner.storage.visit_onboarding_progress().unwrap();
        assert!(schedule::candidates(&mut inner, now())
            .unwrap()
            .iter()
            .all(|p| p.kind != Kind::NoGoals));
        assert!(!valid(&inner, &plan(now()), now()));
        inner.companion.view.preferences.screen_task_detection = false;
        inner.companion.memory.last_prompt = now();
        inner.companion.memory.shown = 100;
        let prompt = inner.storage.onboarding_prompt(now()).unwrap().unwrap();
        enqueue(&mut inner, prompt.clone(), now());
        assert_eq!(deliver(&mut inner, now()).unwrap().unwrap().id, prompt.id);
        inner.status.dnd = true;
        assert!(deliver(&mut inner, now()).unwrap().is_none());
        assert!(inner.companion.view.intervention.is_none());
        inner.status.dnd = false;
        enqueue(&mut inner, prompt.clone(), now());
        assert_eq!(deliver(&mut inner, now()).unwrap().unwrap().id, prompt.id);
    }
    fn state() -> AppState {
        AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap()
    }
    fn now() -> i64 {
        Local::now()
            .date_naive()
            .and_hms_opt(18, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
            .timestamp()
    }
    fn prompt(kind: Kind, id: &str, now: i64) -> Intervention {
        Intervention {
            id: id.into(),
            kind,
            text: "A concrete action".into(),
            goal_id: None,
            plan_revision: None,
            step_id: None,
            confidence: 1.0,
            expires_at: now + 300,
        }
    }
    fn plan(now: i64) -> Intervention {
        prompt(
            Kind::NoGoals,
            &format!(
                "schedule:{}:planning",
                Local.timestamp_opt(now, 0).unwrap().date_naive()
            ),
            now,
        )
    }
    fn new_step(inner: &mut Inner, now: i64) -> Intervention {
        let goal = inner.storage.set_goal("Prepare pitch").unwrap();
        let mut p = prompt(Kind::NewTask, "new-step", now);
        p.goal_id = Some(goal.id);
        p.plan_revision = Some(inner.storage.goal_plan(goal.id).unwrap().revision);
        p
    }
    fn deadline(inner: &mut Inner, due: i64, now: i64) -> Intervention {
        let goal = inner.storage.set_goal("Send pitch").unwrap();
        inner.storage.adopt_core_goals().unwrap();
        inner
            .storage
            .connection
            .execute(
                "INSERT OR REPLACE INTO core_goal_details(goal_id,due_at) VALUES(?1,?2)",
                rusqlite::params![
                    goal.id,
                    chrono::DateTime::from_timestamp(due, 0)
                        .unwrap()
                        .to_rfc3339()
                ],
            )
            .unwrap();
        deadlines::next(
            &inner.storage,
            &inner.companion.memory,
            Some(goal.id),
            "",
            now,
        )
        .unwrap()
        .unwrap()
    }
    #[test]
    fn simultaneous_scheduled_messages_all_reach_the_queue_before_priority_selection() {
        use std::time::{Duration, SystemTime};
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        for n in 0..=900 {
            i.companion
                .work_clock
                .observe_at(start + Duration::from_secs(n * 3), true);
        }
        let candidates = schedule::candidates(&mut i, now).unwrap();
        assert_eq!(candidates.len(), 3);
        for p in candidates {
            enqueue(&mut i, p, now);
        }
        for expected in [Kind::EndOfDay, Kind::Movement, Kind::NoGoals] {
            assert_eq!(deliver(&mut i, now).unwrap().unwrap().kind, expected);
            i.companion.view.intervention = None;
        }
    }
    #[test]
    fn interrupted_planning_never_reappears_without_an_explicit_reminder_request() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        enqueue(&mut i, plan(now), now);
        deliver(&mut i, now).unwrap();
        let due = deadline(&mut i, now, now);
        enqueue(&mut i, due, now);
        deliver(&mut i, now).unwrap();
        i.companion.view.intervention = None;
        assert!(deliver(&mut i, now + 1).unwrap().is_none());
        assert!(i.companion.memory.planning_reminder_at.is_none());
    }
    #[test]
    fn due_deadline_preempts_and_then_restores_a_valid_stretch_card() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        enqueue(&mut i, prompt(Kind::Movement, "stretch", now), now);
        assert_eq!(deliver(&mut i, now).unwrap().unwrap().kind, Kind::Movement);
        let deadline = deadline(&mut i, now, now);
        enqueue(&mut i, deadline, now);
        assert_eq!(deliver(&mut i, now).unwrap().unwrap().kind, Kind::Deadline);
        i.companion.view.intervention = None;
        assert_eq!(
            deliver(&mut i, now + 1).unwrap().unwrap().kind,
            Kind::Movement
        );
        assert_eq!(i.companion.memory.shown, 2); // Resumption is not a second delivery.
    }
    #[test]
    fn queue_retains_questions_during_chat_and_discards_expired_or_stale_evidence() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        i.companion.view.chat_open = true;
        enqueue(&mut i, prompt(Kind::Movement, "stretch", now), now);
        assert!(deliver(&mut i, now).unwrap().is_none());
        i.companion.view.chat_open = false;
        assert_eq!(
            deliver(&mut i, now + 1).unwrap().unwrap().kind,
            Kind::Movement
        );
        i.companion.view.intervention = None;
        enqueue(&mut i, prompt(Kind::Movement, "expired", now), now);
        assert!(deliver(&mut i, now + 301).unwrap().is_none());
        let p = new_step(&mut i, now + 301);
        enqueue(&mut i, p, now + 301);
        i.storage.set_goal("Different goal").unwrap();
        assert!(deliver(&mut i, now + 302).unwrap().is_none());
        assert!(i.companion.queue.items.is_empty());
    }
    #[test]
    fn current_deadline_phase_replaces_a_queued_previous_phase_and_reserves_the_new_phase() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        let p = deadline(&mut i, now + 60, now);
        enqueue(&mut i, p, now);
        assert!(deliver(&mut i, now)
            .unwrap()
            .unwrap()
            .id
            .ends_with(":before"));
        i.companion.view.chat_open = true;
        deliver(&mut i, now + 1).unwrap();
        let p = deadlines::next(&i.storage, &i.companion.memory, None, "", now + 60)
            .unwrap()
            .unwrap();
        enqueue(&mut i, p, now + 60);
        i.companion.view.chat_open = false;
        assert!(deliver(&mut i, now + 60)
            .unwrap()
            .unwrap()
            .id
            .ends_with(":now"));
        assert!(
            deadlines::next(&i.storage, &i.companion.memory, None, "", now + 60)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn ordinary_questions_deduplicate_and_typing_delays_them_at_most_ten_seconds() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        i.buddy.foreground = Some(ActivitySnapshot {
            idle_seconds: 0,
            ..Default::default()
        });
        let p = new_step(&mut i, now);
        enqueue(&mut i, p.clone(), now);
        assert!(deliver(&mut i, now).unwrap().is_none());
        enqueue(&mut i, p.clone(), now + 9);
        assert_eq!(i.companion.queue.items.len(), 1);
        assert!(deliver(&mut i, now + 9).unwrap().is_none());
        assert_eq!(deliver(&mut i, now + 10).unwrap().unwrap().id, p.id);
    }
    #[test]
    fn media_exclusions_and_typing_do_not_block_stretches_but_meetings_hold_the_card() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        i.buddy.foreground = Some(ActivitySnapshot {
            process_name: "private.exe".into(),
            idle_seconds: 0,
            media_playing: true,
            ..Default::default()
        });
        i.buddy.view.preferences.excluded_apps = vec!["private.exe".into()];
        assert!(!super::super::screen_context::allowed(
            &i,
            i.buddy.foreground.as_ref().unwrap()
        ));
        enqueue(&mut i, prompt(Kind::Movement, "stretch", now), now);
        assert!(deliver(&mut i, now).unwrap().is_some());
        i.buddy.foreground.as_mut().unwrap().window_title = "Zoom Meeting".into();
        assert!(deliver(&mut i, now + 1).unwrap().is_none());
        assert!(i.companion.view.intervention.is_none());
        i.buddy.foreground.as_mut().unwrap().window_title.clear();
        assert!(deliver(&mut i, now + 2).unwrap().is_some());
        assert_eq!(i.companion.memory.shown, 1);
    }
    #[test]
    fn quiet_drops_queued_messages_and_overrides_a_due_deadline() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        let p = deadline(&mut i, now, now);
        enqueue(&mut i, p, now);
        i.status.dnd = true;
        assert!(deliver(&mut i, now).unwrap().is_none());
        i.status.dnd = false;
        assert!(deliver(&mut i, now + 1).unwrap().is_none());
    }
    #[test]
    fn a_completion_receipt_waits_for_quiet_mode_to_end_instead_of_being_dropped() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        i.storage
            .connection
            .execute(
                "INSERT INTO work_events(evidence_key,title,app,occurred_at,source,status) VALUES('receipt','Join the call','zoom.us','2026-11-10T10:00:00Z','automatic_verified','completed')",
                [],
            )
            .unwrap();
        let id = i.storage.connection.last_insert_rowid();
        let receipt = Intervention {
            expires_at: now + 86400,
            ..prompt(Kind::AutomaticCompletion, &format!("work-event-{id}"), now)
        };
        // Queued during Do not disturb, and shown on screen when quiet mode starts.
        i.status.dnd = true;
        enqueue(&mut i, receipt.clone(), now);
        enqueue(&mut i, prompt(Kind::Movement, "stretch", now), now);
        assert!(deliver(&mut i, now).unwrap().is_none());
        i.status.dnd = false;
        let shown = deliver(&mut i, now + 1).unwrap().unwrap();
        assert_eq!(
            shown.id, receipt.id,
            "the receipt survived; the question did not"
        );
        i.status.dnd = true;
        assert!(deliver(&mut i, now + 2).unwrap().is_none());
        assert!(i.companion.view.intervention.is_none());
        i.status.dnd = false;
        assert_eq!(deliver(&mut i, now + 3).unwrap().unwrap().id, receipt.id);
    }
    #[test]
    fn all_card_types_have_one_explicit_priority_order() {
        let now = now();
        let cases = [
            (Kind::Deadline, "deadline:1:0:now"),
            (Kind::Completion, "completion"),
            (Kind::EndOfDay, "summary"),
            (Kind::Deadline, "deadline:1:0:before"),
            (Kind::GoalMatch, "match"),
            (Kind::Movement, "stretch"),
            (Kind::NoGoals, "plan"),
        ];
        let priorities: Vec<_> = cases
            .into_iter()
            .map(|(kind, id)| priority(&prompt(kind, id, now)))
            .collect();
        assert_eq!(priorities, vec![0, 1, 2, 3, 4, 5, 6]);
    }
    #[test]
    fn delivery_orders_all_pending_messages_and_revalidates_consent_and_deadlines() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        let due = deadline(&mut i, now, now);
        let step = new_step(&mut i, now);
        let completion = Intervention {
            kind: Kind::Completion,
            id: "completion".into(),
            ..step.clone()
        };
        enqueue(&mut i, plan(now), now);
        enqueue(&mut i, prompt(Kind::Movement, "stretch", now), now);
        enqueue(&mut i, step, now);
        enqueue(&mut i, prompt(Kind::EndOfDay, "summary", now), now);
        enqueue(&mut i, completion, now);
        enqueue(&mut i, due, now);
        for expected in [
            Kind::Deadline,
            Kind::Completion,
            Kind::EndOfDay,
            Kind::NewTask,
            Kind::Movement,
            Kind::NoGoals,
        ] {
            assert_eq!(deliver(&mut i, now).unwrap().unwrap().kind, expected);
            i.companion.view.intervention = None;
        }
        let mut step = new_step(&mut i, now);
        step.id = "consent-step".into();
        enqueue(&mut i, step, now);
        assert_eq!(i.companion.queue.items.len(), 1);
        i.mark_consent_change();
        assert!(deliver(&mut i, now).unwrap().is_none());
        let due = deadline(&mut i, now, now);
        let goal = due.goal_id.unwrap();
        enqueue(&mut i, due, now);
        i.storage
            .connection
            .execute(
                "UPDATE core_goal_details SET due_at=NULL WHERE goal_id=?1",
                [goal],
            )
            .unwrap();
        assert!(deliver(&mut i, now).unwrap().is_none());
    }
    #[test]
    fn storage_failure_does_not_consume_or_lose_a_pending_card() {
        let state = state();
        let mut i = state.inner.lock().unwrap();
        let now = now();
        enqueue(&mut i, plan(now), now);
        i.storage.connection.execute_batch("CREATE TEMP TRIGGER fail_delivery BEFORE INSERT ON preferences WHEN NEW.name='companion_memory' BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;").unwrap();
        assert!(deliver(&mut i, now).is_err());
        assert!(i.companion.view.intervention.is_none());
        assert_eq!(i.companion.memory.shown, 0);
        assert_eq!(i.companion.queue.items.len(), 1);
        i.storage
            .connection
            .execute_batch("DROP TRIGGER fail_delivery")
            .unwrap();
        assert_eq!(
            deliver(&mut i, now + 1).unwrap().unwrap().kind,
            Kind::NoGoals
        );
    }
}

#[cfg(test)]
mod clock_tests {
    use crate::{commands::AppState, models::ActivitySnapshot, storage::Storage};
    use std::time::{Duration, SystemTime};
    #[test]
    fn meeting_time_continues_the_stretch_clock_even_without_keyboard_input() {
        let state =
            AppState::new(Storage::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
        let mut i = state.inner.lock().unwrap();
        let now = SystemTime::now();
        let start = now - Duration::from_secs(900);
        for n in 0..=300 {
            i.companion
                .work_clock
                .observe_at(start + Duration::from_secs(n * 3), true);
        }
        let mut sample = ActivitySnapshot {
            process_name: "zoom.us".into(),
            window_title: "Zoom Meeting".into(),
            idle_seconds: 999,
            ..Default::default()
        };
        super::super::observe(&mut i, &sample, true);
        assert!(i.companion.work_clock.seconds() >= 900);
        sample.window_title = "Music and work".into();
        sample.media_playing = true;
        sample.idle_seconds = 0;
        super::super::observe(&mut i, &sample, true);
        assert!(i.companion.work_clock.seconds() >= 900);
        // Excluded-app content stays unobserved, without throwing away the stretch interval.
        super::super::observe(&mut i, &sample, false);
        assert!(i.companion.view.activity.is_none());
        assert!(i.companion.work_clock.seconds() >= 900);
        sample.idle_seconds = 300;
        super::super::observe(&mut i, &sample, true);
        assert_eq!(i.companion.work_clock.seconds(), 0);
    }
}
