use super::*;
fn store() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}
fn draft(title: &str, area: &str) -> GoalDraft {
    GoalDraft {
        title: title.into(),
        area: area.into(),
        steps: vec!["Review result".into()],
    }
}
#[test]
fn planning_previous_goals_saves_the_whole_selection_and_retries_without_duplicates() {
    let mut s = store();
    s.core_add_goals(
        vec![
            draft("First", "Work"),
            draft("Second", "Work"),
            draft("Existing", "Work"),
        ],
        "picker",
    )
    .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    let ids: Vec<_> = snap.goals.iter().map(|goal| goal.id).collect();
    s.core_today(ids[2], true, &snap.date).unwrap();
    s.core_plan_goals(&ids[..2], &snap.date).unwrap();
    s.core_plan_goals(&[ids[0], ids[1], ids[0]], &snap.date)
        .unwrap();
    let next = s.core_snapshot(None).unwrap();
    assert_eq!(next.today.len(), 3);
    assert!(ids.iter().all(|id| next.today.contains(id)));
    assert_eq!(next.day_mode, "plan");
}

#[test]
fn stale_previous_goal_rolls_back_the_entire_selection() {
    let mut s = store();
    s.core_add_goals(
        vec![draft("Open", "Work"), draft("Closed", "Work")],
        "picker",
    )
    .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    let ids: Vec<_> = snap.goals.iter().map(|goal| goal.id).collect();
    s.core_transition(ids[1], "complete").unwrap();
    s.core_plan_day(&snap.date, true).unwrap();
    for selection in [vec![ids[0], ids[1]], vec![ids[0], i64::MAX], vec![]] {
        assert!(s.core_plan_goals(&selection, &snap.date).is_err());
        let next = s.core_snapshot(None).unwrap();
        assert!(next.today.is_empty());
        assert_eq!(next.day_mode, "no_plan");
    }
}

#[test]
fn goal_order_is_persistent_and_rejects_stale_or_duplicate_lists() {
    let mut storage = store();
    storage
        .core_add_goals(
            vec![
                draft("First", "Work"),
                draft("Second", "Work"),
                draft("Third", "Personal"),
            ],
            "order",
        )
        .unwrap();
    let ids: Vec<_> = storage
        .core_snapshot(None)
        .unwrap()
        .goals
        .iter()
        .map(|goal| goal.id)
        .collect();
    let order = vec![ids[2], ids[0], ids[1]];
    storage.core_reorder(order.clone()).unwrap();
    assert_eq!(
        storage.read_setting::<Vec<i64>>("core_goal_order").unwrap(),
        Some(order.clone())
    );
    assert_eq!(
        storage
            .core_snapshot(None)
            .unwrap()
            .goals
            .iter()
            .map(|goal| goal.id)
            .collect::<Vec<_>>(),
        order
    );
    assert!(storage.core_reorder(vec![ids[0], ids[0], ids[1]]).is_err());
    assert!(storage.core_reorder(vec![ids[0], ids[1]]).is_err());
    storage
        .core_add_goals(vec![draft("New", "Work")], "new-order")
        .unwrap();
    assert!(storage.core_reorder(order.clone()).is_err());
    let current = storage.core_snapshot(None).unwrap();
    assert_eq!(
        current
            .goals
            .iter()
            .take(3)
            .map(|goal| goal.id)
            .collect::<Vec<_>>(),
        order
    );
}
#[test]
fn deadlines_are_atomic_revisioned_and_survive_other_plan_updates() {
    let mut s = store();
    s.core_add_goals(vec![draft("Ship", "Work")], "deadline")
        .unwrap();
    let first = s.core_snapshot(None).unwrap().goals.remove(0);
    let details = GoalDetails {
        due_at: Some("2026-10-10T17:00:00+03:00".into()),
        priority: Some("high".into()),
        description: "Release the Windows app".into(),
    };
    s.core_save_goal_details("Ship safely", "Work", first.plan.clone(), Some(details))
        .unwrap();
    let current = s.core_snapshot(None).unwrap().goals.remove(0);
    assert_eq!(current.due_at.as_deref(), Some("2026-10-10T14:00:00+00:00"));
    assert_eq!(current.priority.as_deref(), Some("high"));
    assert!(!current.created_at.is_empty());
    assert!(s
        .core_save_goal_details("Stale", "Work", first.plan, Some(GoalDetails::default()))
        .is_err());
    s.core_save_goal("Ship safely", "Work", current.plan.clone())
        .unwrap();
    let current = s.core_snapshot(None).unwrap().goals.remove(0);
    assert_eq!(current.priority.as_deref(), Some("high"));
    assert!(s
        .core_save_goal_details(
            "Invalid",
            "Work",
            current.plan.clone(),
            Some(GoalDetails {
                due_at: Some("tomorrow".into()),
                ..Default::default()
            })
        )
        .is_err());
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].title, "Ship safely");
    s.core_transition(current.id, "complete").unwrap();
    assert!(s.core_snapshot(None).unwrap().goals[0]
        .completed_at
        .is_some());
    s.core_transition(current.id, "resume").unwrap();
    assert!(s.core_snapshot(None).unwrap().goals[0]
        .completed_at
        .is_none());
    let current = s.core_snapshot(None).unwrap().goals.remove(0);
    s.core_save_goal_details(
        &current.title,
        "Work",
        current.plan,
        Some(GoalDetails::default()),
    )
    .unwrap();
    assert!(s.core_snapshot(None).unwrap().goals[0].due_at.is_none());
}
#[test]
fn carryover_is_opt_in_and_dismissal_is_persistent_for_only_one_day() {
    let mut s = store();
    s.core_create_goal("Yesterday", "carry-one", "2026-10-01")
        .unwrap();
    let id = s.core_snapshot(None).unwrap().goals[0].id;
    assert_eq!(s.core_carryover("2026-10-02").unwrap(), vec![id]);
    assert!(s.core_goals_for_date("2026-10-02").unwrap().is_empty());
    s.core_dismiss_carryover(id, "2026-10-02").unwrap();
    s.initialize_core().unwrap();
    assert!(s.core_carryover("2026-10-02").unwrap().is_empty());
    assert_eq!(s.core_carryover("2026-10-03").unwrap(), vec![id]);
    s.core_today(id, true, "2026-10-03").unwrap();
    assert!(s.core_carryover("2026-10-03").unwrap().is_empty());
    s.core_transition(id, "complete").unwrap();
    assert!(s.core_carryover("2026-10-04").unwrap().is_empty());
    assert_eq!(s.core_goals_for_date("2026-10-01").unwrap(), vec![id]);
    let historical = s.core_snapshot(Some("2026-10-01")).unwrap();
    assert_eq!(historical.selected_date, "2026-10-01");
    assert_eq!(historical.date_goals, vec![id]);
    assert_eq!(historical.date, Local::now().date_naive().to_string());
    s.core_transition(id, "resume").unwrap();
    s.core_transition(id, "defer").unwrap();
    assert!(s.core_carryover("2026-10-04").unwrap().is_empty());
}
#[test]
fn quick_goal_retries_never_select_an_unrelated_goal_and_today_has_no_limit() {
    let mut s = store();
    for i in 0..4 {
        s.core_create_goal(&format!("Goal {i}"), &format!("quick-{i}"), "2026-10-01")
            .unwrap();
    }
    s.core_create_goal("Retry", "quick-0", "2026-10-02")
        .unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals.len(), 4);
    assert_eq!(s.core_goals_for_date("2026-10-01").unwrap().len(), 4);
    assert!(s.core_goals_for_date("2026-10-02").unwrap().is_empty());
    let id = s.core_snapshot(None).unwrap().goals[0].id;
    s.core_transition(id, "delete").unwrap();
    s.core_create_goal("Deleted retry", "quick-3", "2026-10-02")
        .unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals.len(), 3);
}
#[test]
fn new_goal_metadata_and_dismissal_survive_database_reopening() {
    let path = std::env::temp_dir().join(format!(
        "buddy-goal-core-{}-{}.db",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let id;
    {
        let mut s = Storage::open(&path).unwrap();
        s.core_create_goal("Persistent", "persist-create", "2026-10-01")
            .unwrap();
        let goal = s.core_snapshot(None).unwrap().goals.remove(0);
        id = goal.id;
        s.core_save_goal_details(
            &goal.title,
            "General",
            goal.plan,
            Some(GoalDetails {
                due_at: Some("2026-10-10T12:00:00Z".into()),
                priority: Some("medium".into()),
                description: "Context".into(),
            }),
        )
        .unwrap();
        s.core_dismiss_carryover(id, "2026-10-02").unwrap();
    }
    {
        let s = Storage::open(&path).unwrap();
        let goal = s.core_snapshot(None).unwrap().goals.remove(0);
        assert_eq!(goal.id, id);
        assert_eq!(goal.description, "Context");
        assert_eq!(goal.priority.as_deref(), Some("medium"));
        assert!(goal.due_at.is_some());
        assert!(s.core_carryover("2026-10-02").unwrap().is_empty());
        assert_eq!(s.core_goals_for_date("2026-10-01").unwrap(), vec![id]);
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn parallel_goals_and_today_are_persisted_without_replacing_the_legacy_focus() {
    let mut s = store();
    let legacy = s.set_goal("Original focus").unwrap();
    s.core_add_goals(
        vec![
            draft("Ship", "Work"),
            draft("Read", "Learning"),
            draft("Walk", "Personal"),
        ],
        "batch-1",
    )
    .unwrap();
    s.core_add_goals(vec![draft("Duplicate retry", "Work")], "batch-1")
        .unwrap();
    assert_eq!(s.goal().unwrap().unwrap().id, legacy.id);
    let snap = s.core_snapshot(None).unwrap();
    assert_eq!(snap.goals.len(), 4);
    let open: Vec<_> = snap
        .goals
        .iter()
        .filter(|g| g.status == "open")
        .map(|g| g.id)
        .collect();
    // Today holds as many goals as the person wants.
    for id in &open {
        s.core_today(*id, true, &snap.date).unwrap();
    }
    assert_eq!(s.core_snapshot(None).unwrap().today.len(), open.len());
    s.core_plan_day(&snap.date, true).unwrap();
    assert!(s.core_snapshot(None).unwrap().today.is_empty());
    assert_eq!(s.core_snapshot(None).unwrap().day_mode, "no_plan");
    assert!(s.core_snapshot(Some("2025-02-30")).is_err());
}
#[test]
fn steps_use_revisions_and_finishing_every_step_completes_the_goal() {
    let mut s = store();
    s.core_add_goals(vec![draft("Plan", "Work")], "one")
        .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    let g = &snap.goals[0];
    let mut plan = g.plan.clone();
    plan.steps[0].done = true;
    s.core_save_goal(&g.title, "Work", plan.clone()).unwrap();
    assert!(s.core_save_goal("Stale", "Work", plan).is_err());
    let snap = s.core_snapshot(None).unwrap();
    // The only step is done, so the goal completes by itself.
    assert_eq!(snap.goals[0].status, "completed");
    assert_eq!(snap.summary.completed_goals, vec!["Plan"]);
    s.core_transition(g.id, "resume").unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "open");
    let mut plan = s.core_snapshot(None).unwrap().goals[0].plan.clone();
    plan.steps[0].done = false;
    s.core_save_goal("Plan", "Work", plan).unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "open");
    s.core_transition(g.id, "complete").unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "completed");
    s.core_transition(g.id, "resume").unwrap();
    assert!(s.core_snapshot(None).unwrap().summary.completed.is_empty());
    s.core_transition(g.id, "delete").unwrap();
    assert!(s.core_snapshot(None).unwrap().goals.is_empty());
}
#[test]
fn focus_timer_skips_sleep_and_restart_and_never_counts_another_goal() {
    let mut s = store();
    s.core_add_goals(vec![draft("Timer", "Work")], "timer")
        .unwrap();
    let id = s.core_snapshot(None).unwrap().goals[0].id;
    let now = Utc::now();
    s.write_setting(
        "core_timer",
        &Timer {
            goal_id: id,
            last_tick: now,
        },
    )
    .unwrap();
    s.core_tick_at(now + Duration::seconds(15)).unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].focused_seconds, 15);
    s.core_tick_at(now + Duration::hours(2)).unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].focused_seconds, 15);
    s.initialize_core().unwrap();
    assert!(s.core_timer().unwrap().is_none());
}
#[test]
fn import_validation_is_atomic_and_goal_deletion_handles_existing_foreign_keys() {
    let mut s = store();
    assert!(s
        .core_add_goals(vec![draft("Valid", "Work"), draft("", "Work")], "bad")
        .is_err());
    assert!(s.core_snapshot(None).unwrap().goals.is_empty());
    let goal = s.set_goal("Legacy").unwrap();
    s.core_snapshot(None).unwrap();
    let activity = crate::models::ActivitySnapshot {
        timestamp: Utc::now().to_rfc3339(),
        process_name: "fixture.exe".into(),
        window_title: "Fixture".into(),
        idle_seconds: 0,
        active_seconds: 1,
        window_id: None,
        browser: None,
        media_playing: false,
        document_key: None,
    };
    s.activity(goal.id, &activity).unwrap();
    s.core_transition(goal.id, "delete").unwrap();
    assert!(s.goal().unwrap().is_none());
}
#[test]
fn companion_plan_changes_share_revisions_and_progress_with_core_goals() {
    let mut s = store();
    let goal = s.set_goal("Shared goal").unwrap();
    s.core_snapshot(None).unwrap();
    let mut plan = s.goal_plan(goal.id).unwrap();
    plan.steps.push(Step {
        id: "buddy-fixture".into(),
        text: "Send the deck".into(),
        done: false,
    });
    s.save_goal_plan(&goal.text, plan).unwrap();
    let snapshot = s.core_snapshot(None).unwrap();
    let stale = snapshot.goals[0].plan.clone();
    let mut completed = stale.clone();
    completed.steps[0].done = true;
    s.save_goal_plan(&goal.text, completed).unwrap();
    assert_eq!(
        s.core_snapshot(None).unwrap().summary.completed,
        vec!["Send the deck"]
    );
    assert!(s.core_save_goal("Stale edit", "General", stale).is_err());
    s.core_set_timer(Some(goal.id)).unwrap();
    s.transition_goal(goal.id, "defer").unwrap();
    let snapshot = s.core_snapshot(None).unwrap();
    assert_eq!(snapshot.goals[0].status, "deferred");
    assert!(snapshot.timer.is_none());
    s.transition_goal(goal.id, "resume").unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "open");
}
#[test]
fn clear_history_keeps_parallel_goals_and_clears_progress_and_timer() {
    let mut s = store();
    s.core_add_goals(vec![draft("Keep me", "Work")], "keep")
        .unwrap();
    let goal = &s.core_snapshot(None).unwrap().goals[0];
    let now = Utc::now();
    s.write_setting(
        "core_timer",
        &Timer {
            goal_id: goal.id,
            last_tick: now,
        },
    )
    .unwrap();
    s.core_tick_at(now + Duration::milliseconds(2900)).unwrap();
    s.core_tick_at(now + Duration::milliseconds(5900)).unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].focused_seconds, 5);
    s.purge_history(None).unwrap();
    let snapshot = s.core_snapshot(None).unwrap();
    assert_eq!(snapshot.goals.len(), 1);
    assert_eq!(snapshot.goals[0].focused_seconds, 0);
    assert!(snapshot.timer.is_none());
    assert!(snapshot.summary.completed.is_empty());
}
#[test]
fn completing_a_goal_completes_its_steps_and_reopening_starts_them_over() {
    let mut storage = store();
    storage
        .core_add_goals(vec![draft("Ship", "Work")], "complete-steps")
        .unwrap();
    let id = storage.core_snapshot(None).unwrap().goals[0].id;
    assert!(!storage.goal_plan(id).unwrap().steps[0].done);
    storage.core_transition(id, "complete").unwrap();
    assert!(storage.goal_plan(id).unwrap().steps.iter().all(|s| s.done));
    let summary = storage.core_snapshot(None).unwrap().summary;
    assert!(summary.completed.contains(&"Review result".to_string()));
    assert_eq!(summary.completed_goals, vec!["Ship".to_string()]);
    storage.core_transition(id, "resume").unwrap();
    assert!(storage.goal_plan(id).unwrap().steps.iter().all(|s| !s.done));
    let summary = storage.core_snapshot(None).unwrap().summary;
    assert!(summary.completed_goals.is_empty());
}
#[test]
fn settling_completes_open_goals_whose_steps_are_all_done() {
    let mut s = store();
    s.core_add_goals(vec![draft("Late", "Work")], "settle")
        .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    let (gid, mut plan) = (snap.goals[0].id, snap.goals[0].plan.clone());
    plan.steps[0].done = true;
    // Simulate older data: steps done while the goal stayed open.
    s.connection
        .execute(
            "UPDATE goal_plans SET payload=?1 WHERE goal_id=?2",
            params![serde_json::to_string(&plan).unwrap(), gid],
        )
        .unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "open");
    s.core_settle_goals().unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "completed");
}
#[test]
fn unticking_a_step_reopens_a_completed_goal_without_resetting_other_steps() {
    let mut s = store();
    s.core_add_goals(
        vec![GoalDraft {
            title: "Two".into(),
            area: "Work".into(),
            steps: vec!["One".into(), "Two".into()],
        }],
        "untick",
    )
    .unwrap();
    let id = s.core_snapshot(None).unwrap().goals[0].id;
    s.core_transition(id, "complete").unwrap();
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "completed");
    let mut plan = s.core_snapshot(None).unwrap().goals[0].plan.clone();
    plan.steps[1].done = false;
    s.core_save_goal("Two", "Work", plan).unwrap();
    let snap = s.core_snapshot(None).unwrap();
    assert_eq!(snap.goals[0].status, "open");
    assert!(snap.goals[0].plan.steps[0].done);
    assert!(!snap.goals[0].plan.steps[1].done);
}
#[test]
fn relevant_time_is_attributed_to_the_current_open_step() {
    let mut s = store();
    s.core_add_goals(
        vec![GoalDraft {
            title: "Timed".into(),
            area: "Work".into(),
            steps: vec!["First".into(), "Second".into()],
        }],
        "step-time",
    )
    .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    let goal = &snap.goals[0];
    let (first, second) = (goal.plan.steps[0].id.clone(), goal.plan.steps[1].id.clone());
    // With nothing chosen, the first unfinished step gets the time.
    assert_eq!(s.attributed_step(goal.id).unwrap(), first);
    let mut plan = goal.plan.clone();
    plan.current_step = Some(second.clone());
    s.core_save_goal("Timed", "Work", plan).unwrap();
    assert_eq!(s.attributed_step(goal.id).unwrap(), second);
    let mut plan = s.core_snapshot(None).unwrap().goals[0].plan.clone();
    plan.steps[1].done = true;
    plan.current_step = None;
    s.core_save_goal("Timed", "Work", plan).unwrap();
    // The second step is done, so the first one is the open step again.
    assert_eq!(s.attributed_step(goal.id).unwrap(), first);
    s.connection
        .execute(
            "INSERT INTO goal_relevant_step_daily(day,goal_id,step_id,milliseconds) VALUES('2026-10-12',?1,?2,90000)",
            params![goal.id, first],
        )
        .unwrap();
    let times = s.step_times(goal.id, "2026-10-12").unwrap();
    assert_eq!(times.len(), 1);
    assert_eq!(times[0].seconds, 90);
}
#[test]
fn periods_cover_whole_calendar_months_and_years_ending_at_the_anchor() {
    let s = store();
    let length = |anchor: &str, period: &str| {
        s.core_snapshot_period(Some(anchor), Some(period))
            .unwrap()
            .week
            .len()
    };
    assert_eq!(length("2026-02-28", "month"), 28);
    assert_eq!(length("2026-09-30", "month"), 30);
    assert_eq!(length("2025-12-31", "year"), 365);
    assert_eq!(length("2026-10-07", "week"), 7);
    assert_eq!(length("2026-10-07", "day"), 1);
}

#[test]
fn today_drag_order_survives_database_reopen_without_changing_today_membership() {
    let path = std::env::temp_dir().join(format!(
        "buddy-today-order-{}-{}.sqlite",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let expected;
    let today;
    let date = chrono::Local::now().date_naive().to_string();
    {
        let mut storage = Storage::open(&path).unwrap();
        storage
            .core_add_goals(
                vec![
                    draft("First", "Work"),
                    draft("Other day", "Personal"),
                    draft("Second", "Work"),
                    draft("Third", "Work"),
                ],
                "drag-order",
            )
            .unwrap();
        let ids: Vec<i64> = storage
            .core_snapshot(None)
            .unwrap()
            .goals
            .iter()
            .map(|g| g.id)
            .collect();
        today = vec![ids[0], ids[2], ids[3]];
        for id in &ids {
            storage.core_today(*id, today.contains(id), &date).unwrap();
        }
        expected = vec![ids[2], ids[1], ids[3], ids[0]];
        storage.core_reorder(expected.clone()).unwrap();
    }
    {
        let storage = Storage::open(&path).unwrap();
        let snapshot = storage.core_snapshot(Some(&date)).unwrap();
        assert_eq!(
            snapshot.goals.iter().map(|g| g.id).collect::<Vec<_>>(),
            expected
        );
        let mut actual = snapshot.today;
        actual.sort();
        let mut membership = today;
        membership.sort();
        assert_eq!(actual, membership);
    }
    std::fs::remove_file(path).unwrap();
}

fn celebration(storage: &Storage) -> u64 {
    storage
        .read_setting("mascot_completion_revision")
        .unwrap()
        .unwrap_or(0)
}
#[test]
fn manual_step_and_goal_completions_publish_a_durable_non_replaying_signal() {
    let mut s = store();
    let mut goal = draft("Animation test", "Work");
    goal.steps.push("Send result".into());
    s.core_add_goals(vec![goal], "animation").unwrap();
    let id = s.core_snapshot(None).unwrap().goals[0].id;
    assert_eq!(celebration(&s), 0);
    let mut plan = s.goal_plan(id).unwrap();
    plan.steps[0].done = true;
    s.core_save_goal_details("Animation test", "Work", plan, None)
        .unwrap();
    assert_eq!(celebration(&s), 1);
    s.core_transition(id, "complete").unwrap();
    let completed = celebration(&s);
    assert!(completed > 1);
    s.core_transition(id, "complete").unwrap();
    assert_eq!(celebration(&s), completed);
    s.core_transition(id, "resume").unwrap();
    assert_eq!(celebration(&s), completed);
    s.core_transition(id, "complete").unwrap();
    assert!(celebration(&s) > completed);
}
#[test]
fn restoring_completed_history_does_not_publish_a_new_achievement() {
    let mut s = store();
    s.core_add_goals(
        vec![draft("Historical result", "Work")],
        "animation-restore",
    )
    .unwrap();
    let id = s.core_snapshot(None).unwrap().goals[0].id;
    s.core_transition(id, "complete").unwrap();
    let completed = celebration(&s);
    let backup = crate::skills::delete(&mut s, id).unwrap();
    assert_eq!(celebration(&s), completed);
    crate::skills::restore(&mut s, backup).unwrap();
    assert_eq!(celebration(&s), completed);
}
#[test]
fn a_rolled_back_completion_does_not_celebrate() {
    let mut s = store();
    s.core_add_goals(vec![draft("Rollback", "Work")], "animation-rollback")
        .unwrap();
    let id = s.core_snapshot(None).unwrap().goals[0].id;
    s.connection
        .execute_batch("SAVEPOINT completion_test")
        .unwrap();
    s.core_transition(id, "complete").unwrap();
    assert!(celebration(&s) > 0);
    s.connection
        .execute_batch("ROLLBACK TO completion_test; RELEASE completion_test")
        .unwrap();
    assert_eq!(celebration(&s), 0);
}
#[test]
fn autosaved_stretch_preferences_update_both_gates_without_changing_quiet_consent() {
    let mut s = store();
    let companion = crate::companion::engine::Preferences {
        quiet_hours_enabled: true,
        use_screenshots: false,
        ..Default::default()
    };
    s.write_setting("companion_preferences", &companion)
        .unwrap();
    let mut core = CorePreferences {
        movement_reminders: false,
        ..Default::default()
    };
    let result = persist_preferences(&mut s, &core).unwrap();
    assert!(!result.movement_reminder);
    assert!(result.quiet_hours_enabled);
    assert!(!result.use_screenshots);
    core.movement_reminders = true;
    core.movement_interval_minutes = 15;
    assert!(
        persist_preferences(&mut s, &core)
            .unwrap()
            .movement_reminder
    );
    let saved: CorePreferences = s.read_setting("core_preferences").unwrap().unwrap();
    assert!(saved.movement_reminders);
    assert_eq!(saved.movement_interval_minutes, 15);
}
#[test]
fn failed_second_stretch_write_rolls_back_the_first() {
    let mut s = store();
    let core = CorePreferences::default();
    persist_preferences(&mut s, &core).unwrap();
    s.connection.execute_batch("CREATE TRIGGER reject_companion_update BEFORE UPDATE ON preferences WHEN NEW.name='companion_preferences' BEGIN SELECT RAISE(ABORT,'test storage failure'); END;").unwrap();
    let next = CorePreferences {
        movement_reminders: !core.movement_reminders,
        ..core.clone()
    };
    assert!(persist_preferences(&mut s, &next).is_err());
    let saved: CorePreferences = s.read_setting("core_preferences").unwrap().unwrap();
    assert_eq!(saved.movement_reminders, core.movement_reminders);
}
