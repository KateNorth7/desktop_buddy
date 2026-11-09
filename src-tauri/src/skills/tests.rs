use super::*;

fn store() -> Storage {
    Storage::open(std::path::Path::new(":memory:")).unwrap()
}

#[test]
fn conversation_read_uses_request_scope_even_after_ui_switch_and_without_goal_sharing() {
    let mut storage = store();
    let conversation = storage.current_conversation().unwrap();
    let reply = ai_chat::ChatReply {
        message: "Understood".into(),
        ..Default::default()
    };
    storage
        .save_chat_turn("Original chat fact", &reply, None)
        .unwrap();
    let snap = snapshot(&storage, conversation).unwrap();
    let other = storage.create_conversation().unwrap();
    storage
        .save_chat_turn("Private other chat", &reply, None)
        .unwrap();
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions: vec![action(
            "inspect_conversation",
            json!({"query":"", "before_message_id":null}),
        )],
        clarification: None,
    };
    let result = execute(
        &mut storage,
        &snap,
        &plan,
        "Read our earlier conversation",
        false,
        false,
    )
    .unwrap();
    assert!(!result.committed_changes());
    let messages = result.results[0].data["messages"].to_string();
    assert!(messages.contains("Original chat fact"));
    assert!(!messages.contains("Private other chat"));
    assert_eq!(storage.current_conversation().unwrap(), other);
    for input in [
        json!({"query":"", "before_message_id":0}),
        json!({"query":"", "before_message_id":null,"conversation_id":other}),
        json!({"query":"x".repeat(201),"before_message_id":null}),
    ] {
        let skill = registry()
            .into_iter()
            .find(|s| s.name == "inspect_conversation")
            .unwrap();
        assert!((skill.validator)(&skill, &input).is_err());
    }
}
fn action(name: &str, input: Value) -> Action {
    Action {
        name: name.into(),
        input,
        instruction: Some("Do this".into()),
    }
}
fn goal(id: i64) -> Value {
    json!({"goal_id":id,"reference":null})
}
fn reference(name: &str) -> Value {
    json!({"goal_id":null,"reference":name})
}
fn step(id: &str) -> Value {
    json!({"step_id":id,"ordinal":null,"reference":null})
}
fn ordinal(n: usize) -> Value {
    json!({"step_id":null,"ordinal":n,"reference":null})
}
fn run(store: &mut Storage, actions: Vec<Action>) -> Execution {
    let conversation = store.current_conversation().unwrap();
    let snap = snapshot(store, conversation).unwrap();
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions,
        clarification: None,
    };
    let out = execute(store, &snap, &plan, "Do this", true, false).unwrap();
    if out.status == "confirmation_required"
        && plan.actions.iter().all(|a| {
            a.name == "complete_goal"
                || registry()
                    .iter()
                    .any(|s| s.name == a.name && s.safety != Safety::Confirmation)
        })
    {
        return pending_reply(store, conversation, "Confirm", true, 0)
            .unwrap()
            .unwrap();
    }
    out
}
fn create(store: &mut Storage, title: &str, steps: Vec<&str>) -> i64 {
    run(
        store,
        vec![action("create_goal", json!({"title":title,"steps":steps}))],
    )
    .results[0]
        .goal_id
        .unwrap()
}
fn fail(store: &mut Storage, actions: Vec<Action>) -> String {
    let conv = store.current_conversation().unwrap();
    let snap = snapshot(store, conv).unwrap();
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions,
        clarification: None,
    };
    execute(store, &snap, &plan, "Do this", true, false)
        .err()
        .unwrap()
}

#[test]
fn acceptance_ordinals_include_completed_steps_in_the_full_order() {
    let mut s = store();
    let id = create(
        &mut s,
        "Ordinal acceptance",
        vec!["Already done", "Second step", "Third step"],
    );
    let first_id = s.goal_plan(id).unwrap().steps[0].id.clone();
    run(
        &mut s,
        vec![action(
            "complete_step",
            json!({"goal":goal(id),"step":step(&first_id)}),
        )],
    );
    run(
        &mut s,
        vec![action(
            "rename_step",
            json!({"goal":goal(id),"step":ordinal(2),"text":"Renamed second step"}),
        )],
    );
    let plan = s.goal_plan(id).unwrap();
    assert!(plan.steps[0].done);
    assert_eq!(plan.steps[0].text, "Already done");
    assert_eq!(plan.steps[1].text, "Renamed second step");
    assert_eq!(plan.steps[2].text, "Third step");
    run(
        &mut s,
        vec![action(
            "reopen_step",
            json!({"goal":goal(id),"step":ordinal(1)}),
        )],
    );
    assert!(!s.goal_plan(id).unwrap().steps[0].done);
}

#[test]
fn create_goal_through_structured_plan_and_result() {
    let mut s = store();
    let c = s.current_conversation().unwrap();
    let snap = snapshot(&s, c).unwrap();
    let response = json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"kind":"actions","state_version":snap.version,"actions":[{"name":"create_goal","input":{"title":"Prepare for interview","steps":[]},"instruction":"Create a goal"}],"clarification":null}).to_string()}}]});
    let plan = parse_plan(&response).unwrap();
    let out = execute(&mut s, &snap, &plan, "Create a goal", true, false).unwrap();
    assert_eq!(out.status, "executed");
    let id = out.results[0].goal_id.unwrap();
    let core = s.core_snapshot(None).unwrap();
    assert!(core.today.contains(&id));
    assert_eq!(core.goals[0].title, "Prepare for interview");
    assert_eq!(
        s.action_references(c).unwrap().last_created_goal_id,
        Some(id)
    );
}
#[test]
fn rename_goal_uses_id_even_when_names_are_identical() {
    let mut s = store();
    let first = create(&mut s, "Interview", vec![]);
    let second = create(&mut s, "Interview", vec![]);
    run(
        &mut s,
        vec![action(
            "rename_goal",
            json!({"goal":goal(first),"title":"First interview"}),
        )],
    );
    let core = s.core_snapshot(None).unwrap();
    assert_eq!(
        core.goals.iter().find(|g| g.id == first).unwrap().title,
        "First interview"
    );
    assert_eq!(
        core.goals.iter().find(|g| g.id == second).unwrap().title,
        "Interview"
    );
}
#[test]
fn create_step_returns_actual_id_and_records_conversation_reference() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec![]);
    let out = run(
        &mut s,
        vec![action(
            "create_step",
            json!({"goal":goal(id),"text":"Review the company"}),
        )],
    );
    let step_id = out.results[0].step_id.as_ref().unwrap();
    assert_eq!(s.goal_plan(id).unwrap().steps[0].id, *step_id);
    let c = s.current_conversation().unwrap();
    assert_eq!(
        s.action_references(c)
            .unwrap()
            .last_created_step_id
            .as_ref(),
        Some(step_id)
    );
}
#[test]
fn complete_and_reopen_step_reuse_goal_completion_rules() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["Read"]);
    let sid = s.goal_plan(id).unwrap().steps[0].id.clone();
    run(
        &mut s,
        vec![action(
            "complete_step",
            json!({"goal":goal(id),"step":step(&sid)}),
        )],
    );
    assert!(s.goal_plan(id).unwrap().steps[0].done);
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "completed");
    run(
        &mut s,
        vec![action(
            "reopen_step",
            json!({"goal":goal(id),"step":step(&sid)}),
        )],
    );
    assert!(!s.goal_plan(id).unwrap().steps[0].done);
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "open");
}
#[test]
fn delete_restore_preserves_step_ids_time_and_completion_events() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["Read", "Practice"]);
    let sid = s.goal_plan(id).unwrap().steps[0].id.clone();
    s.connection.execute("INSERT INTO core_goal_details(goal_id,description,priority) VALUES(?1,'Keep this description','high')",[id]).unwrap();
    s.connection
        .execute(
            "INSERT INTO app_rules(goal_id,process_name,category) VALUES(?1,'editor','work')",
            [id],
        )
        .unwrap();
    run(
        &mut s,
        vec![action(
            "complete_step",
            json!({"goal":goal(id),"step":step(&sid)}),
        )],
    );
    s.connection
        .execute(
            "INSERT INTO core_time(day,goal_id,seconds) VALUES('2026-10-12',?1,123)",
            [id],
        )
        .unwrap();
    let out = run(
        &mut s,
        vec![action("delete_goal", json!({"goal":goal(id)}))],
    );
    assert!(s.core_snapshot(None).unwrap().goals.is_empty());
    let backup = out.results[0].data["backup_id"].as_i64().unwrap();
    let restored = run(
        &mut s,
        vec![action("restore_goal", json!({"backup_id":backup}))],
    )
    .results[0]
        .goal_id
        .unwrap();
    assert_eq!(
        s.core_snapshot(None).unwrap().goals[0].description,
        "Keep this description"
    );
    assert_eq!(
        s.connection
            .query_row(
                "SELECT category FROM app_rules WHERE goal_id=?1",
                [restored],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "work"
    );
    assert_eq!(s.goal_plan(restored).unwrap().steps[0].id, sid);
    assert!(s.goal_plan(restored).unwrap().steps[0].done);
    assert_eq!(
        s.connection
            .query_row(
                "SELECT seconds FROM core_time WHERE goal_id=?1",
                [restored],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        123
    );
    assert_eq!(
        s.connection
            .query_row(
                "SELECT COUNT(*) FROM core_events WHERE goal_id=?1",
                [restored],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}
#[test]
fn invalid_goal_id_never_falls_back_to_current_goal() {
    let mut s = store();
    create(&mut s, "Keep", vec![]);
    assert!(fail(
        &mut s,
        vec![action(
            "rename_goal",
            json!({"goal":goal(999999),"title":"Wrong"})
        )]
    )
    .contains("no longer exists"));
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].title, "Keep");
}
#[test]
fn invalid_step_id_is_rejected_without_text_matching() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["Read"]);
    assert!(fail(
        &mut s,
        vec![action(
            "complete_step",
            json!({"goal":goal(id),"step":step("Read")})
        )]
    )
    .contains("does not belong"));
    assert!(!s.goal_plan(id).unwrap().steps[0].done);
}
#[test]
fn step_from_another_goal_is_rejected() {
    let mut s = store();
    let a = create(&mut s, "A", vec![]);
    let b = create(&mut s, "B", vec![]);
    let sid = run(
        &mut s,
        vec![action("create_step", json!({"goal":goal(a),"text":"Read"}))],
    )
    .results[0]
        .step_id
        .clone()
        .unwrap();
    assert!(fail(
        &mut s,
        vec![action(
            "complete_step",
            json!({"goal":goal(b),"step":step(&sid)})
        )]
    )
    .contains("does not belong"));
    assert!(!s.goal_plan(a).unwrap().steps[0].done);
}
#[test]
fn multi_action_command_can_reference_new_objects_without_invented_ids() {
    let mut s = store();
    let out = run(
        &mut s,
        vec![
            action("create_goal", json!({"title":"Interview","steps":[]})),
            action(
                "create_step",
                json!({"goal":reference("previous_action"),"text":"Read"}),
            ),
            action(
                "rename_step",
                json!({"goal":reference("previous_action"),"step":{"step_id":null,"ordinal":null,"reference":"previous_action"},"text":"Read the company profile"}),
            ),
        ],
    );
    assert_eq!(out.results.len(), 3);
    let id = out.results[0].goal_id.unwrap();
    assert_eq!(
        s.goal_plan(id).unwrap().steps[0].text,
        "Read the company profile"
    );
}
#[test]
fn add_there_uses_last_referenced_goal_across_messages() {
    let mut s = store();
    let id = create(&mut s, "Prepare for interview", vec![]);
    let mut actions = vec![];
    for text in [
        "Research company",
        "Prepare examples",
        "Practice answers",
        "Prepare questions",
    ] {
        let mut a = action(
            "create_step",
            json!({"goal":reference("last_referenced"),"text":text}),
        );
        a.instruction = Some("Добавь туда четыре шага".into());
        actions.push(a);
    }
    let c = s.current_conversation().unwrap();
    let snap = snapshot(&s, c).unwrap();
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions,
        clarification: None,
    };
    execute(&mut s, &snap, &plan, "Добавь туда четыре шага", true, false).unwrap();
    assert_eq!(s.goal_plan(id).unwrap().steps.len(), 4);
}
#[test]
fn delete_second_then_complete_first_requires_exact_confirmation() {
    let mut s = store();
    let id = create(
        &mut s,
        "Interview",
        vec!["Research", "Practice", "Questions"],
    );
    let mut delete = action(
        "delete_step",
        json!({"goal":reference("last_referenced"),"step":ordinal(2)}),
    );
    delete.instruction = Some("Второй удали".into());
    let c = s.current_conversation().unwrap();
    let snap = snapshot(&s, c).unwrap();
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions: vec![delete],
        clarification: None,
    };
    let pending = execute(&mut s, &snap, &plan, "Второй удали", true, false).unwrap();
    assert_eq!(pending.status, "confirmation_required");
    assert_eq!(s.goal_plan(id).unwrap().steps.len(), 3);
    assert_eq!(pending.results[0].data["removed_step"]["text"], "Practice");
    assert_eq!(
        pending_reply(&mut s, c, "Подтверждаю", true, 0)
            .unwrap()
            .unwrap()
            .status,
        "executed"
    );
    run(
        &mut s,
        vec![action(
            "complete_step",
            json!({"goal":reference("last_referenced"),"step":ordinal(1)}),
        )],
    );
    let plan = s.goal_plan(id).unwrap();
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[1].text, "Questions");
    assert!(plan.steps[0].done);
}
#[test]
fn ambiguous_reference_requests_clarification_without_mutation() {
    let mut s = store();
    create(&mut s, "Interview", vec![]);
    create(&mut s, "Interview", vec![]);
    let c = s.create_conversation().unwrap();
    assert!(fail(
        &mut s,
        vec![action(
            "create_step",
            json!({"goal":reference("only_goal"),"text":"Read"})
        )]
    )
    .contains("Which goal"));
    assert!(fail(
        &mut s,
        vec![action(
            "create_step",
            json!({"goal":reference("last_referenced"),"text":"Read"})
        )]
    )
    .contains("Which goal"));
    assert_eq!(s.action_references(c).unwrap(), References::default());
    assert!(s
        .core_snapshot(None)
        .unwrap()
        .goals
        .iter()
        .all(|g| g.plan.steps.is_empty()));
}
#[test]
fn stale_state_rejects_entire_plan() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec![]);
    let c = s.current_conversation().unwrap();
    let snap = snapshot(&s, c).unwrap();
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions: vec![action(
            "create_step",
            json!({"goal":goal(id),"text":"Read"}),
        )],
        clarification: None,
    };
    s.connection
        .execute(
            "UPDATE goals SET text='Changed elsewhere' WHERE id=?1",
            [id],
        )
        .unwrap();
    assert!(execute(&mut s, &snap, &plan, "Do this", true, false).is_err());
    assert!(s.goal_plan(id).unwrap().steps.is_empty());
}

#[test]
fn state_revision_rejects_change_then_change_back() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec![]);
    let c = s.current_conversation().unwrap();
    let snap = snapshot(&s, c).unwrap();
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions: vec![action("rename_goal", json!({"goal":goal(id),"title":"No"}))],
        clarification: None,
    };
    s.connection
        .execute("UPDATE goals SET text='Other' WHERE id=?1", [id])
        .unwrap();
    s.connection
        .execute("UPDATE goals SET text='Interview' WHERE id=?1", [id])
        .unwrap();
    assert!(execute(&mut s, &snap, &plan, "Do this", true, false).is_err());
}

#[test]
fn old_conversation_reference_never_targets_reused_sqlite_goal_id() {
    let mut s = store();
    let id = create(&mut s, "Original", vec![]);
    let c = s.current_conversation().unwrap();
    s.create_conversation().unwrap();
    s.core_transition(id, "delete").unwrap();
    let replacement = create(&mut s, "Replacement", vec![]);
    assert_eq!(id, replacement);
    s.select_conversation(c).unwrap();
    assert!(fail(
        &mut s,
        vec![action(
            "rename_goal",
            json!({"goal":reference("last_created"),"title":"Wrong"})
        )]
    )
    .contains("Which goal"));
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].title, "Replacement");
}
#[test]
fn malformed_model_json_unknown_skills_and_extra_fields_are_rejected() {
    for content in [
        "not json",
        r#"{"kind":"actions"}"#,
        r#"{"kind":"chat","state_version":"x","actions":[],"clarification":null,"extra":true}"#,
        r#"{"kind":"actions","state_version":"x","actions":[{"name":"run_shell","input":{},"instruction":"Do this"}],"clarification":null}"#,
    ] {
        assert!(parse_plan(
            &json!({"choices":[{"finish_reason":"stop","message":{"content":content}}]})
        )
        .is_err());
    }
    let s = registry()
        .into_iter()
        .find(|s| s.name == "create_goal")
        .unwrap();
    for input in [
        json!({"title":"Read","steps":[],"goal_id":12}),
        json!({"title":null,"steps":[]}),
        json!({"title":"Read","steps":"wrong"}),
    ] {
        assert!((s.validator)(&s, &input).is_err());
    }
}
#[test]
fn batch_rolls_back_goal_creation_and_references_on_later_failure() {
    let mut s = store();
    let c = s.current_conversation().unwrap();
    fail(
        &mut s,
        vec![
            action("create_goal", json!({"title":"Must roll back","steps":[]})),
            action("rename_goal", json!({"goal":goal(9999),"title":"Invalid"})),
        ],
    );
    assert!(s.core_snapshot(None).unwrap().goals.is_empty());
    assert_eq!(s.action_references(c).unwrap(), References::default());
}
#[test]
fn confirmation_is_invalidated_by_state_change_and_cannot_be_replayed() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["One", "Two"]);
    run(
        &mut s,
        vec![action(
            "delete_step",
            json!({"goal":goal(id),"step":ordinal(2)}),
        )],
    );
    let c = s.current_conversation().unwrap();
    run(
        &mut s,
        vec![action(
            "rename_goal",
            json!({"goal":goal(id),"title":"Changed"}),
        )],
    );
    assert!(pending_reply(&mut s, c, "Confirm", true, 0).is_err());
    assert!(pending_reply(&mut s, c, "Confirm", true, 0)
        .unwrap()
        .is_none());
    assert_eq!(s.goal_plan(id).unwrap().steps.len(), 2);
}
#[test]
fn confirmation_respects_privacy_revocation_and_cancellation() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["One", "Two"]);
    let c = s.current_conversation().unwrap();
    let mut snap = snapshot(&s, c).unwrap();
    snap.privacy_revision = Some(4);
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions: vec![action(
            "delete_step",
            json!({"goal":goal(id),"step":ordinal(2)}),
        )],
        clarification: None,
    };
    execute(&mut s, &snap, &plan, "Do this", true, false).unwrap();
    assert!(pending_reply(&mut s, c, "Confirm", true, 5).is_err());
    assert_eq!(s.goal_plan(id).unwrap().steps.len(), 2);
    run(&mut s, plan.actions);
    assert_eq!(
        pending_reply(&mut s, c, "Cancel", true, 5)
            .unwrap()
            .unwrap()
            .status,
        "cancelled"
    );
}
#[test]
fn last_created_step_reference_checks_owner_even_with_same_local_step_id() {
    let mut s = store();
    let a = create(&mut s, "A", vec![]);
    let b = create(&mut s, "B", vec![]);
    run(
        &mut s,
        vec![action("create_step", json!({"goal":goal(a),"text":"Read"}))],
    );
    assert!(fail(&mut s,vec![action("complete_step",json!({"goal":goal(b),"step":{"step_id":null,"ordinal":null,"reference":"last_created"}}))]).contains("Which step"));
}
#[test]
fn read_skills_today_and_focus_share_the_same_registry() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec![]);
    let out = run(
        &mut s,
        vec![
            action("list_goals", json!({})),
            action("inspect_goal", json!({"goal":goal(id)})),
            action("set_focus_goal", json!({"goal":goal(id)})),
            action("remove_goal_from_today", json!({"goal":goal(id)})),
            action("add_goal_to_today", json!({"goal":goal(id)})),
        ],
    );
    assert_eq!(out.results.len(), 5);
    assert_eq!(out.results[0].data["goals"][0]["goal_id"], id);
    assert!(s.core_snapshot(None).unwrap().today.contains(&id));
}
#[test]
fn goal_sharing_off_blocks_existing_objects_and_sends_no_reference_state() {
    let mut s = store();
    let id = create(&mut s, "Private", vec![]);
    let c = s.current_conversation().unwrap();
    let snap = snapshot(&s, c).unwrap();
    assert!(!planning_context(&snap, false)
        .to_string()
        .contains("Private"));
    let plan = Plan {
        kind: "actions".into(),
        state_version: snap.version.clone(),
        actions: vec![action("inspect_goal", json!({"goal":goal(id)}))],
        clarification: None,
    };
    assert!(execute(&mut s, &snap, &plan, "Do this", false, false).is_err());
}
#[test]
fn conversation_references_survive_reopen_and_are_deleted_with_chat() {
    let path = std::env::temp_dir().join(format!(
        "buddy-skills-{}-{}.sqlite",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let (c, id) = {
        let mut s = Storage::open(&path).unwrap();
        let id = create(&mut s, "Interview", vec![]);
        (s.current_conversation().unwrap(), id)
    };
    {
        let mut s = Storage::open(&path).unwrap();
        assert_eq!(
            s.action_references(c).unwrap().last_created_goal_id,
            Some(id)
        );
        s.delete_conversation(c).unwrap();
        assert_eq!(s.action_references(c).unwrap(), References::default());
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn unquoted_mutation_and_out_of_range_ordinal_do_not_change_state() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["One"]);
    let mut a = action("rename_goal", json!({"goal":goal(id),"title":"No"}));
    a.instruction = Some("Words not in the message".into());
    assert!(fail(&mut s, vec![a]).contains("explicit instruction"));
    assert!(fail(
        &mut s,
        vec![action(
            "complete_step",
            json!({"goal":goal(id),"step":ordinal(2)})
        )]
    )
    .contains("Which step"));
}

#[test]
fn every_skill_exposes_validator_executor_and_strict_result_contract() {
    for skill in registry() {
        assert!(!skill.description.is_empty());
        assert_eq!(skill.input_schema["additionalProperties"], false);
        assert_eq!(skill.result_schema["additionalProperties"], false);
        assert!(!valid_result(
            &skill.result_schema,
            &json!({"skill":skill.name,"goal_id":null,"step_id":null,"data":{"invented":true}})
        ));
    }
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["One"]);
    run(
        &mut s,
        vec![action("complete_goal", json!({"goal":goal(id)}))],
    );
    assert_eq!(s.core_snapshot(None).unwrap().goals[0].status, "completed");
    run(
        &mut s,
        vec![action("reopen_goal", json!({"goal":goal(id)}))],
    );
    assert!(s.goal_plan(id).unwrap().steps[0].done);
}

#[test]
fn result_fallback_is_readable_without_internal_json_or_identifiers() {
    let mut s = store();
    let id = create(&mut s, "Interview", vec!["One", "Two"]);
    let preview = run(
        &mut s,
        vec![action(
            "delete_step",
            json!({"goal":goal(id),"step":ordinal(2)}),
        )],
    );
    let text = preview.fallback_message();
    assert!(text.contains("Two"));
    assert!(text.contains("Confirm or Cancel"));
    assert!(!text.contains("goal_id"));
    assert!(!text.contains("step-1"));
}

#[tokio::test]
async fn malformed_provider_output_retries_then_rejects_without_changes() {
    use wiremock::{matchers::method, Mock, MockServer};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(crate::claude::test::reply(
            json!({"choices":[{"finish_reason":"stop","message":{"content":"malformed"}}]}),
        ))
        .expect(2)
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let service = crate::goal_analysis::Service {
        client: &client,
        endpoint: server.uri(),
        key: "fixture".into(),
        model: "fixture-model".into(),
    };
    assert!(service
        .request_validated(
            "Plan",
            json!({}),
            "buddy_skill_plan",
            plan_schema(),
            parse_plan
        )
        .await
        .is_err());
}

#[test]
fn continuity_read_skill_validates_goal_view_and_result_contract() {
    let mut s = store();
    let id = create(&mut s, "Continuity fixture", vec!["Review proposal"]);
    let e = run(
        &mut s,
        vec![action(
            "inspect_continuity",
            json!({"view":"remaining","goal":goal(id)}),
        )],
    );
    assert_eq!(e.results[0].goal_id, Some(id));
    assert!(e.fallback_message().contains("Review proposal"));
    assert!(!e.committed_changes());
    assert!(!fail(
        &mut s,
        vec![action(
            "inspect_continuity",
            json!({"view":"remaining","goal":goal(999999)})
        )]
    )
    .is_empty());
    assert!(!fail(
        &mut s,
        vec![action(
            "inspect_continuity",
            json!({"view":"fabricate","goal":null})
        )]
    )
    .is_empty());
    assert!(fail(
        &mut s,
        vec![action(
            "inspect_continuity",
            json!({"view":"remaining","goal":null})
        )]
    )
    .contains("Which goal"));
}

#[test]
fn creating_after_park_uses_the_new_batch_reference_and_rolls_back_wrong_targets() {
    let mut storage = store();
    let old = create(&mut storage, "Existing parked goal", vec![]);
    storage.core_transition(old, "defer").unwrap();
    let rejected = fail(
        &mut storage,
        vec![
            action(
                "create_goal",
                json!({"title":"Fresh goal","steps":["Send reply"]}),
            ),
            action("add_goal_to_today", json!({"goal":goal(old)})),
        ],
    );
    assert!(rejected.contains("creation result"));
    assert_eq!(storage.core_snapshot(None).unwrap().goals.len(), 1);
    let out = run(
        &mut storage,
        vec![
            action(
                "create_goal",
                json!({"title":"Fresh goal","steps":["Send reply"]}),
            ),
            action(
                "add_goal_to_today",
                json!({"goal":reference("last_created")}),
            ),
            action("set_focus_goal", json!({"goal":reference("last_created")})),
        ],
    );
    let new = out.results[0].goal_id.unwrap();
    let snapshot = storage.core_snapshot(None).unwrap();
    assert_ne!(new, old);
    assert_eq!(snapshot.focused_goal_id, Some(new));
    assert!(snapshot.today.contains(&new));
    assert_eq!(
        snapshot.goals.iter().find(|g| g.id == old).unwrap().status,
        "deferred"
    );
    assert_eq!(
        snapshot.goals.iter().find(|g| g.id == new).unwrap().title,
        "Fresh goal"
    );
    assert_eq!(storage.goal_plan(new).unwrap().steps.len(), 1);
}

#[test]
fn creation_never_treats_an_open_existing_id_as_the_new_goal() {
    let mut storage = store();
    let old = create(&mut storage, "Existing open goal", vec![]);
    let message = fail(
        &mut storage,
        vec![
            action("create_goal", json!({"title":"Another goal","steps":[]})),
            action("set_focus_goal", json!({"goal":goal(old)})),
        ],
    );
    assert!(message.contains("reference=last_created"));
    assert_eq!(storage.core_snapshot(None).unwrap().goals.len(), 1);
    let out = run(
        &mut storage,
        vec![
            action("create_goal", json!({"title":"Another goal","steps":[]})),
            action("inspect_goal", json!({"goal":goal(old)})),
            action("set_focus_goal", json!({"goal":reference("last_created")})),
        ],
    );
    assert_eq!(
        storage.core_snapshot(None).unwrap().focused_goal_id,
        out.results[0].goal_id
    );
    // A deliberate existing target remains supported before creation.
    let out = run(
        &mut storage,
        vec![
            action("set_focus_goal", json!({"goal":goal(old)})),
            action("create_goal", json!({"title":"Third goal","steps":[]})),
        ],
    );
    assert_eq!(out.status, "executed");
    assert_eq!(
        storage.core_snapshot(None).unwrap().focused_goal_id,
        Some(old)
    );
}
