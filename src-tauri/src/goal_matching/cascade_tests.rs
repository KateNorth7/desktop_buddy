use super::*;
use crate::{
    commands::Inner,
    core::GoalDraft,
    storage::Storage,
    work_state::{self, State},
};
pub(super) fn setup(names: &[&str]) -> (AppState, Vec<i64>) {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    for (i, name) in names.iter().enumerate() {
        s.core_add_goals(
            vec![GoalDraft {
                title: (*name).into(),
                area: "Work".into(),
                steps: vec![format!("Deliver {name}")],
            }],
            &format!("lane-{i}"),
        )
        .unwrap();
    }
    let snap = s.core_snapshot(None).unwrap();
    let ids = names
        .iter()
        .map(|name| snap.goals.iter().find(|g| &g.title == name).unwrap().id)
        .collect::<Vec<_>>();
    for id in &ids {
        s.core_today(*id, true, &snap.date).unwrap();
    }
    s.transition_goal(ids[0], "resume").unwrap();
    s.write_setting(
        "ai_assistance_preferences",
        &crate::goal_analysis::AiPreferences {
            automatic_goal_matching: true,
            ..Default::default()
        },
    )
    .unwrap();
    let state = AppState::new(s).unwrap();
    {
        let mut i = state.inner.lock().unwrap();
        i.status.tracking = true;
        crate::tracking::allow_notifications(&mut i);
        i.status.anthropic_configured = true;
        i.status.mock_ai = false;
        i.companion.view.preferences.quiet_start_minute = 0;
        i.companion.view.preferences.quiet_end_minute = 0;
    }
    (state, ids)
}
pub(super) fn sample(title: &str) -> ActivitySnapshot {
    ActivitySnapshot {
        timestamp: chrono::Utc::now().to_rfc3339(),
        process_name: "Editor".into(),
        window_title: title.into(),
        idle_seconds: 1,
        ..Default::default()
    }
}
pub(super) fn feed(inner: &mut Inner, a: &ActivitySnapshot, base: Instant, start: u64, end: u64) {
    for t in (start..=end).step_by(3) {
        observe_inner_at(inner, a, true, base + Duration::from_secs(t)).unwrap();
    }
}
fn checkpoint(inner: &mut Inner, id: i64, app: &str, waiting: Option<&str>) {
    inner
        .storage
        .park_work_context(
            id,
            crate::work_context::CheckpointDraft {
                summary: "Requirements identified".into(),
                next_action: "Write proposal".into(),
                waiting_for: waiting.map(str::to_string),
            },
            Some(app.into()),
        )
        .unwrap();
    inner.storage.resume_work_context(id, false).unwrap();
}
#[test]
fn explicit_focus_wins_without_inflating_other_goal_activity() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    i.storage.mark_explicit_focus(ids[0]).unwrap();
    let a = sample("Vega budget workbook");
    observe_inner_at(&mut i, &a, true, Instant::now()).unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[0]));
    assert!(!due_inner(&i, Instant::now() + AI_STABLE));
    assert_eq!(attribution(&i, &a).unwrap().confidence, 0.);
    let view = work_state::compute(&i).unwrap();
    assert_eq!(view.active_goal_id, Some(ids[0]));
    assert_eq!(view.state, State::Drifting);
}
#[test]
fn explicit_focus_keeps_the_chosen_lane_but_generic_activity_remains_pending() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    i.storage.mark_explicit_focus(ids[0]).unwrap();
    // A meeting, an inbox or a generic document never contain goal keywords.
    for title in [
        "Meet - Weekly sync",
        "Inbox (3) - Mail",
        "Untitled document",
    ] {
        let a = sample(title);
        observe_inner_at(&mut i, &a, true, Instant::now()).unwrap();
        assert_eq!(i.goal_matching.view.goal_id, Some(ids[0]));
        let matched = attribution(&i, &a).unwrap();
        assert_eq!(
            matched.goal_id.as_deref(),
            Some(ids[0].to_string().as_str())
        );
        assert_eq!(matched.confidence, 0., "{title}");
    }
    // Several goals matching at once is ambiguous and stays out of the selected goal's time.
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
    let mut i = state.inner.lock().unwrap();
    i.storage.mark_explicit_focus(ids[0]).unwrap();
    let a = sample("Atlas launch");
    observe_inner_at(&mut i, &a, true, Instant::now()).unwrap();
    assert_eq!(attribution(&i, &a).unwrap().confidence, 0.);
}
#[test]
fn a_sustained_lane_switch_parks_the_previous_goal_and_returning_resumes_it() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    feed(&mut i, &sample("Orion launch deck"), base, 0, 33);
    assert_eq!(i.storage.goal().unwrap().map(|g| g.id), Some(ids[0]));
    feed(&mut i, &sample("Vega budget workbook"), base, 36, 96);
    assert_eq!(i.storage.goal().unwrap().map(|g| g.id), Some(ids[1]));
    let parked = i.storage.work_context(ids[0]).unwrap().expect("parked");
    assert!(parked
        .context
        .summary
        .starts_with("Automatically parked while in Editor"));
    assert_eq!(
        parked.context.next_action.as_deref(),
        Some("Deliver Orion launch")
    );
    assert_eq!(
        parked.context.last_window.as_deref(),
        Some("Orion launch deck")
    );
    assert!(parked.resumed_at.is_none());
    feed(&mut i, &sample("Orion launch deck"), base, 100, 160);
    assert_eq!(i.storage.goal().unwrap().map(|g| g.id), Some(ids[0]));
    assert!(i
        .storage
        .work_context(ids[0])
        .unwrap()
        .unwrap()
        .resumed_at
        .is_some());
}
#[test]
fn unique_user_app_rule_precedes_keywords_and_presets_do_not() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    i.storage.transition_goal(ids[1], "resume").unwrap();
    i.storage
        .save_app_rule(ids[1], "editor", Some(crate::insights::Category::Work))
        .unwrap();
    let a = sample("Orion launch");
    feed(&mut i, &a, Instant::now(), 0, 30);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("user_rule"));
    i.storage
        .connection
        .execute(
            "UPDATE app_rules SET preset_source='fixture-role' WHERE goal_id=?1",
            [ids[1]],
        )
        .unwrap();
    feed(&mut i, &a, Instant::now(), 0, 30);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[0]));
    assert_eq!(
        i.goal_matching.view.source.as_deref(),
        Some("local_keywords")
    );
}
#[test]
fn six_lanes_strong_local_match_is_ready_at_thirty_seconds() {
    let (state, ids) = setup(&[
        "Orion launch",
        "Vega budget",
        "Cobalt audit",
        "Amber hiring",
        "Quartz research",
        "Nimbus design",
    ]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = sample("Quartz research findings");
    feed(&mut i, &a, base, 0, 27);
    assert!(i.goal_matching.view.goal_id.is_none());
    observe_inner_at(&mut i, &a, true, base + LOCAL_STABLE).unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[4]));
    assert_eq!(
        i.goal_matching.view.source.as_deref(),
        Some("local_keywords")
    );
    assert!(!due_inner(&i, base + AI_STABLE));
    assert_eq!(i.goal_matching.metrics.reasoning_calls, 0);
}
#[tokio::test]
async fn ambiguity_reuses_a_match_within_the_evidence_window_then_requires_a_refresh() {
    use wiremock::{matchers::method, Mock, MockServer};
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta", "Vega budget"]);
    let base = Instant::now();
    let a = sample("Atlas launch beta");
    let job = {
        let mut i = state.inner.lock().unwrap();
        feed(&mut i, &a, base, 0, 27);
        assert!(!due_inner(&i, base + Duration::from_secs(27)));
        observe_inner_at(&mut i, &a, true, base + AI_STABLE).unwrap();
        claim(&mut i, base + AI_STABLE).unwrap().unwrap()
    };
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"goal_id":ids[1],"confidence":0.93,"reason":"The exact beta deliverable matches"}).to_string()}}]}))).expect(1).mount(&server).await;
    let result = request_at(
        &state.client,
        &server.uri(),
        "fixture",
        "fixture",
        &job.goals,
        &job.sample,
        false,
        None,
        &job.recent,
    )
    .await;
    let mut i = state.inner.lock().unwrap();
    // The answer lands a minute in; its memory for this window lasts five minutes from then.
    finish(&mut i, &job, result, base + Duration::from_secs(60)).unwrap();
    feed(&mut i, &a, base, 63, 357);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert!(!due_inner(&i, base + Duration::from_secs(357)));
    assert_eq!(i.goal_matching.metrics.reasoning_calls, 1);
    feed(&mut i, &a, base, 360, 363);
    assert!(i.goal_matching.view.goal_id.is_none());
    assert!(due_inner(&i, base + Duration::from_secs(363)));
}
#[test]
fn work_context_disambiguates_similar_goals_and_respects_memory_off() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    checkpoint(&mut i, ids[1], "Editor", None);
    i.storage.connection.execute("UPDATE work_contexts SET summary='Atlas launch requirements identified' WHERE goal_id=?1",[ids[1]]).unwrap();
    let base = Instant::now();
    let a = sample("Atlas launch");
    feed(&mut i, &a, base, 0, 30);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("work_context"));
    assert_eq!(attribution(&i, &a).unwrap().confidence, 0.);
    let mut controls = i.storage.memory_controls().unwrap();
    controls.enabled = false;
    i.storage.set_memory_controls(&controls).unwrap();
    feed(&mut i, &a, base, 33, 96);
    assert!(i.goal_matching.view.goal_id.is_none());
    assert!(work_state::compute(&i)
        .unwrap()
        .current_work_context
        .is_none());
}
#[test]
fn short_switches_do_not_change_lane_but_sustained_switch_does() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    feed(&mut i, &sample("Orion launch"), base, 0, 30);
    for t in (33..93).step_by(3) {
        let a = if (t / 6) % 2 == 0 {
            sample("Vega budget")
        } else {
            sample("Orion launch")
        };
        observe_inner_at(&mut i, &a, true, base + Duration::from_secs(t)).unwrap();
        assert_eq!(i.storage.goal().unwrap().unwrap().id, ids[0]);
    }
    let a = sample("Vega budget");
    feed(&mut i, &a, base, 96, 126);
    assert_eq!(i.storage.goal().unwrap().unwrap().id, ids[1]);
    assert_eq!(
        work_state::compute(&i).unwrap().active_goal_id,
        Some(ids[1])
    );
}
#[test]
fn similar_goals_and_unknown_context_are_never_guessed() {
    let (state, _) = setup(&[
        "Atlas launch alpha",
        "Atlas launch beta",
        "Atlas launch gamma",
    ]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    feed(&mut i, &sample("Atlas launch"), base, 0, 60);
    assert!(i.goal_matching.view.goal_id.is_none());
    assert!(due_inner(&i, base + AI_STABLE));
    feed(&mut i, &sample("Untitled document"), base, 63, 153);
    assert!(i.goal_matching.view.goal_id.is_none());
    assert!(due_inner(&i, base + Duration::from_secs(153)));
    assert_eq!(work_state::compute(&i).unwrap().confidence, 0.);
}
#[test]
fn excluded_private_idle_and_tracking_off_clear_evidence_and_suppress_ai() {
    for gate in 0..4 {
        let (state, _) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
        let mut i = state.inner.lock().unwrap();
        let base = Instant::now();
        let mut a = sample("Atlas launch");
        feed(&mut i, &a, base, 0, 60);
        match gate {
            0 => i.buddy.view.preferences.excluded_apps.push("Editor".into()),
            1 => a.window_title = "Password vault".into(),
            2 => a.idle_seconds = 1000,
            _ => i.status.tracking = false,
        }
        observe_inner_at(&mut i, &a, true, base + Duration::from_secs(63)).unwrap();
        assert!(i.goal_matching.current_sample().is_none());
        assert!(!due_inner(&i, base + Duration::from_secs(200)));
        assert_eq!(work_state::compute(&i).unwrap().state, State::Away);
    }
}
#[test]
fn waiting_context_and_live_step_survive_a_lane_switch_without_tick_memories() {
    let (state, ids) = setup(&["Orion launch", "Quartz research"]);
    let mut i = state.inner.lock().unwrap();
    checkpoint(&mut i, ids[1], "Research", Some("Reviewer feedback"));
    let mut a = sample("Requirements identified");
    a.process_name = "Research".into();
    let base = Instant::now();
    feed(&mut i, &a, base, 0, 30);
    let before: i64 = i
        .storage
        .connection
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    for _ in 0..100 {
        let view = work_state::compute(&i).unwrap();
        assert_eq!(view.active_goal_id, Some(ids[1]));
        assert_eq!(view.state, State::Waiting);
        assert_eq!(view.waiting_for.as_deref(), Some("Reviewer feedback"));
        assert!(view.active_step_id.is_some());
        assert!(!serde_json::to_string(&view)
            .unwrap()
            .contains("last_window"));
    }
    let after: i64 = i
        .storage
        .connection
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
}
#[test]
fn cloud_null_or_failure_is_cached_and_new_privacy_invalidates_it() {
    for fail in [false, true] {
        let (state, _) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
        let mut i = state.inner.lock().unwrap();
        let base = Instant::now();
        let a = sample("Atlas launch");
        feed(&mut i, &a, base, 0, 60);
        let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
        let result = if fail {
            Err("Provider failed".into())
        } else {
            Ok(MatchResult {
                goal_id: None,
                confidence: 0.,
                reason: "Still ambiguous".into(),
                same_task_trail: vec![],
            })
        };
        finish(&mut i, &job, result, base + AI_STABLE).unwrap();
        feed(&mut i, &a, base, 63, 300);
        assert!(!due_inner(&i, base + Duration::from_secs(300)));
        assert_eq!(i.goal_matching.metrics.reasoning_calls, 1);
        i.privacy_revision += 1;
        feed(&mut i, &a, base, 603, 696);
        assert!(due_inner(&i, base + Duration::from_secs(696)));
    }
}
#[test]
fn a_new_explicit_focus_invalidates_an_old_cloud_response() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = sample("Atlas launch");
    feed(&mut i, &a, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    i.storage.mark_explicit_focus(ids[0]).unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.95,
            reason: "Stale".into(),
            same_task_trail: vec![],
        }),
        base + Duration::from_secs(61),
    )
    .unwrap();
    assert_eq!(i.storage.goal().unwrap().unwrap().id, ids[0]);
    assert!(!i.goal_matching.in_flight);
}
#[test]
fn explicit_focus_persists_reopen_and_is_cleared_after_parking() {
    let path = std::env::temp_dir().join(format!(
        "buddy-lane-{}.db",
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let id;
    {
        let mut s = Storage::open(&path).unwrap();
        s.core_add_goals(
            vec![GoalDraft {
                title: "Orion launch".into(),
                area: "Work".into(),
                steps: vec![],
            }],
            "persist-focus",
        )
        .unwrap();
        id = s.core_snapshot(None).unwrap().goals[0].id;
        s.transition_goal(id, "resume").unwrap();
        s.mark_explicit_focus(id).unwrap();
    }
    {
        let mut s = Storage::open(&path).unwrap();
        assert_eq!(s.explicit_focus().unwrap(), Some(id));
        s.core_transition(id, "defer").unwrap();
        assert!(s.explicit_focus().unwrap().is_none());
        s.core_transition(id, "resume").unwrap();
        assert!(s.explicit_focus().unwrap().is_none());
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dnd_and_sharing_block_calls_even_for_sustained_ambiguity() {
    let (state, _) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = sample("Atlas launch");
    feed(&mut i, &a, base, 0, 60);
    assert!(due_inner(&i, base + AI_STABLE));
    i.status.dnd = true;
    assert!(!due_inner(&i, base + AI_STABLE));
    i.status.dnd = false;
    let mut prefs = i.storage.ai_preferences().unwrap();
    prefs.share_goal_context = false;
    i.storage
        .write_setting("ai_assistance_preferences", &prefs)
        .unwrap();
    assert!(!due_inner(&i, base + AI_STABLE));
    prefs.share_goal_context = true;
    i.storage
        .write_setting("ai_assistance_preferences", &prefs)
        .unwrap();
    assert!(due_inner(&i, base + AI_STABLE));
}
#[test]
fn work_state_reports_stuck_and_distraction_without_mutating_memory() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    i.storage
        .connection
        .execute(
            "UPDATE goals SET created_at=?1 WHERE id=?2",
            rusqlite::params![
                (chrono::Utc::now() - chrono::Duration::hours(4)).to_rfc3339(),
                ids[0]
            ],
        )
        .unwrap();
    let base = Instant::now();
    let a = sample("Orion launch");
    feed(&mut i, &a, base, 0, 30);
    assert_eq!(work_state::compute(&i).unwrap().state, State::Stuck);
    i.storage
        .save_app_rule(
            ids[0],
            "Editor",
            Some(crate::insights::Category::Distraction),
        )
        .unwrap();
    i.storage.mark_explicit_focus(ids[0]).unwrap();
    observe_inner_at(&mut i, &a, true, base + Duration::from_secs(33)).unwrap();
    assert_eq!(work_state::compute(&i).unwrap().state, State::Drifting);
}
#[tokio::test]
async fn synthetic_switching_benchmark() {
    use wiremock::{matchers::method, Mock, MockServer};
    let (state, ids) = setup(&[
        "Orion launch",
        "Vega budget",
        "Cobalt audit",
        "Quartz research",
        "Atlas launch alpha",
        "Atlas launch beta",
    ]);
    {
        let mut i = state.inner.lock().unwrap();
        checkpoint(&mut i, ids[3], "Research", None);
    }
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(crate::claude::test::reply(json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"goal_id":ids[5],"confidence":0.93,"reason":"The beta deliverable is visible"}).to_string()}}]}))).expect(1).mount(&server).await;
    let base = Instant::now();
    let mut rows = Vec::new();
    let mut previous_key = String::new();
    let mut old_since = 0;
    let mut old_last: Option<u64> = None;
    let mut old_calls = 0;
    let mut old_resolutions = 0;
    // Six 90-second work segments, separated once by four six-second accidental switches.
    let segments = [
        (0, 90, "Editor", "Orion launch", ids[0]),
        (114, 90, "Editor", "Vega budget", ids[1]),
        (204, 90, "Editor", "Atlas launch beta", ids[5]),
        (294, 90, "Research", "Requirements identified", ids[3]),
        (384, 90, "Editor", "Cobalt audit", ids[2]),
        (474, 90, "Editor", "Orion launch", ids[0]),
    ];
    for t in (0..564).step_by(3) {
        let segment = segments
            .iter()
            .find(|(start, len, _, _, _)| t >= *start && t < start + len);
        let (app, title) = segment.map(|s| (s.2, s.3)).unwrap_or_else(|| {
            if (t / 6) % 2 == 0 {
                ("Editor", "Cobalt audit")
            } else {
                ("Editor", "Vega budget")
            }
        });
        let mut a = sample(title);
        a.process_name = app.into();
        let key = fingerprint(&a);
        if previous_key != key {
            previous_key = key;
            old_since = t;
        }
        if t - old_since >= 600 && old_last.is_none_or(|last| t - last >= 600) {
            old_calls += 1;
            old_resolutions += 1;
            old_last = Some(t);
        }
        let job = {
            let mut i = state.inner.lock().unwrap();
            observe_inner_at(&mut i, &a, true, base + Duration::from_secs(t)).unwrap();
            claim(&mut i, base + Duration::from_secs(t)).unwrap()
        };
        if let Some(job) = job {
            let response = request_at(
                &state.client,
                &server.uri(),
                "fixture",
                "fixture",
                &job.goals,
                &job.sample,
                false,
                None,
                &job.recent,
            )
            .await;
            finish(
                &mut state.inner.lock().unwrap(),
                &job,
                response,
                base + Duration::from_secs(t),
            )
            .unwrap();
        }
        if let Some((start, _, _, _, expected)) = segment {
            let i = state.inner.lock().unwrap();
            if i.goal_matching.view.goal_id == Some(*expected)
                && !rows.iter().any(|r: &Value| r["start"] == *start)
            {
                rows.push(json!({"start":start,"latency_seconds":t-start,"source":i.goal_matching.view.source}));
            }
        }
    }
    let i = state.inner.lock().unwrap();
    assert_eq!(rows.len(), 6);
    assert_eq!(i.goal_matching.metrics.reasoning_calls, 1);
    assert_eq!(i.goal_matching.metrics.local_resolutions, 5);
    assert_eq!(old_resolutions, 0);
    assert_eq!(old_calls, 0);
    assert!(rows
        .iter()
        .all(|r| r["latency_seconds"].as_u64().unwrap() <= 60));
    // Separate stationary reference: old policy first becomes eligible at 600s; new at 60s.
    let report = json!({"scenario_seconds":564,"active_goals":6,"old":{"stable_threshold_seconds":600,"recognized_segments":old_resolutions,"reasoning_calls":old_calls,"local_only_resolutions":0},"new":{"segments":rows,"reasoning_calls":i.goal_matching.metrics.reasoning_calls,"local_only_resolutions":i.goal_matching.metrics.local_resolutions},"stationary_ambiguous_first_eligibility_seconds":{"old":600,"new":AI_STABLE.as_secs()},"note":"Simulated 3-second collection; actual local HTTP fixture; provider latency excluded; counts cover goal matching only"});
    println!("SWITCHING_BENCHMARK {}", report);
}
#[test]
fn returning_to_a_resolved_lane_reuses_the_scoped_ai_result_after_debounce() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let ambiguous = sample("Atlas launch beta");
    feed(&mut i, &ambiguous, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.95,
            reason: "Beta deliverable".into(),
            same_task_trail: vec![],
        }),
        base + AI_STABLE,
    )
    .unwrap();
    feed(&mut i, &sample("Vega budget"), base, 63, 93);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[2]));
    feed(&mut i, &ambiguous, base, 96, 126);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("model"));
    assert_eq!(i.goal_matching.metrics.reasoning_calls, 1);
    assert!(!due_inner(&i, base + Duration::from_secs(200)));
}
#[test]
fn an_ai_match_counts_the_whole_stable_context_and_carries_across_titles() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let wall = chrono::Utc::now();
    let mut a = sample("Research notes");
    a.window_id = Some(7);
    feed(&mut i, &a, base, 0, 60);
    // Collection recorded the context under the current goal without verified relevance.
    let unmatched = crate::relevance::ActivityMatch {
        goal_id: None,
        confidence: 0.,
        reason: "No verified relevance".into(),
    };
    for n in 0..19 {
        let interval = crate::history::UsageInterval {
            start: wall - chrono::Duration::seconds(60 - n * 3),
            end: wall - chrono::Duration::seconds(57 - n * 3),
            activity: a.clone(),
            state: crate::tracking::ActivityState::Focused,
        };
        i.storage
            .record_interval_with_match(ids[0], &interval, Some(unmatched.clone()))
            .unwrap();
        i.storage
            .record_review_interval(&interval, &unmatched)
            .unwrap();
    }
    assert_eq!(i.storage.relevant_seconds(ids[1], None).unwrap(), 0);
    // The answer arrives a minute in; everything since the window appeared counts.
    let job = claim(&mut i, base + Duration::from_secs(60))
        .unwrap()
        .unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.86,
            reason: "The notes draft the beta plan".into(),
            same_task_trail: vec![],
        }),
        base + Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.storage.goal().unwrap().unwrap().id, ids[1]);
    // The minutes before the answer moved to the matched goal with the real confidence.
    assert_eq!(i.storage.relevant_seconds(ids[1], None).unwrap(), 57);
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 0);
    let (count, confidence): (i64, f64) = i
        .storage
        .connection
        .query_row(
            "SELECT COUNT(*),MIN(match_confidence) FROM usage_intervals WHERE goal_id=?1",
            [ids[1]],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 19);
    assert!((confidence - 0.86).abs() < 1e-9);
    let left: i64 = i
        .storage
        .connection
        .query_row(
            "SELECT COALESCE(SUM(milliseconds),0) FROM usage_daily WHERE goal_id=?1",
            [ids[0]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0, "time is moved, never duplicated");
    // A new title in the same window keeps counting while a refresh is requested.
    let mut b = sample("Research notes - beta timeline");
    b.window_id = Some(7);
    feed(&mut i, &b, base, 63, 90);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("model_carry"));
    assert!(attribution(&i, &b).unwrap().confidence >= crate::relevance::THRESHOLD);
    assert!(!due_inner(&i, base + Duration::from_secs(90)));
    feed(&mut i, &b, base, 93, 93);
    assert!(due_inner(&i, base + Duration::from_secs(93)));
    let job = claim(&mut i, base + Duration::from_secs(93))
        .unwrap()
        .unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: None,
            confidence: 0.2,
            reason: "Unrelated notes".into(),
            same_task_trail: vec![],
        }),
        base + Duration::from_secs(93),
    )
    .unwrap();
    assert!(i.goal_matching.view.goal_id.is_none());
    let unassigned = attribution(&i, &b).unwrap();
    assert!(unassigned.goal_id.is_none() && unassigned.confidence < crate::relevance::THRESHOLD);
}
#[test]
fn another_window_or_a_gap_never_inherits_an_ai_match() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let mut a = sample("Research notes");
    a.window_id = Some(7);
    feed(&mut i, &a, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.9,
            reason: "Beta notes".into(),
            same_task_trail: vec![],
        }),
        base + AI_STABLE,
    )
    .unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    let mut other = sample("Other document");
    other.window_id = Some(9);
    observe_inner_at(&mut i, &other, true, base + Duration::from_secs(63)).unwrap();
    assert!(i.goal_matching.view.goal_id.is_none());
    assert_ne!(i.goal_matching.view.source.as_deref(), Some("model_carry"));
    // Returning to the resolved window reuses its answer after the debounce.
    feed(&mut i, &a, base, 66, 100);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    // A gap longer than a missed sample starts the window over instead of carrying.
    observe_inner_at(&mut i, &a, true, base + Duration::from_secs(120)).unwrap();
    assert!(i.goal_matching.view.goal_id.is_none(), "a gap starts over");
    assert_ne!(i.goal_matching.view.source.as_deref(), Some("model_carry"));
}
#[test]
fn a_browser_match_carries_only_within_the_same_domain() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let page = |title: &str, domain: &str| {
        let mut a = sample(title);
        a.process_name = "Google Chrome".into();
        a.window_id = Some(11);
        a.browser = crate::browser::context(&a.process_name, title, Some(domain.into()));
        a
    };
    let first = page("Plan - Notion", "notion.so");
    feed(&mut i, &first, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.9,
            reason: "Beta plan page".into(),
            same_task_trail: vec![],
        }),
        base + AI_STABLE,
    )
    .unwrap();
    feed(
        &mut i,
        &page("Timeline - Notion", "notion.so"),
        base,
        63,
        69,
    );
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("model_carry"));
    feed(
        &mut i,
        &page("Trending - YouTube", "youtube.com"),
        base,
        72,
        78,
    );
    assert!(i.goal_matching.view.goal_id.is_none());
}
#[test]
fn a_recently_worked_lane_wins_a_keyword_tie_but_fresh_lanes_stay_ambiguous() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = sample("Atlas launch");
    // Two lanes created today are equally current: nothing is guessed.
    feed(&mut i, &a, base, 0, 60);
    assert!(i.goal_matching.view.goal_id.is_none());
    assert!(due_inner(&i, base + AI_STABLE));
    // Once the second lane has sat untouched for days, the active lane continues.
    let old = (chrono::Utc::now() - chrono::Duration::days(3)).to_rfc3339();
    i.storage
        .connection
        .execute(
            "UPDATE goals SET created_at=?1 WHERE id=?2",
            rusqlite::params![old, ids[1]],
        )
        .unwrap();
    i.goal_matching.clear();
    feed(&mut i, &a, base, 63, 123);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[0]));
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("local_recent"));
    assert!(attribution(&i, &a).unwrap().confidence >= crate::relevance::THRESHOLD);
    assert!(!due_inner(&i, base + Duration::from_secs(123)));
}
#[test]
fn long_parked_lanes_leave_the_candidates_unless_their_own_words_match() {
    let (state, ids) = setup(&["Atlas launch alpha", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let date = i.storage.core_snapshot(None).unwrap().date;
    i.storage.core_today(ids[1], false, &date).unwrap();
    let old = (chrono::Utc::now() - chrono::Duration::days(20)).to_rfc3339();
    i.storage
        .connection
        .execute(
            "UPDATE goals SET created_at=?1 WHERE id=?2",
            rusqlite::params![old, ids[1]],
        )
        .unwrap();
    let listed = |goals: &Value| {
        goals
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["id"].as_i64().unwrap())
            .collect::<Vec<_>>()
    };
    let generic = candidates(&i, &sample("Research notes")).unwrap();
    assert_eq!(listed(&generic), vec![ids[0]]);
    assert_eq!(generic[0]["lane"]["active"], json!(true));
    assert_eq!(generic[0]["lane"]["in_today"], json!(true));
    assert_eq!(generic[0]["lane"]["last_worked_days_ago"], json!(0));
    let own = candidates(&i, &sample("Vega budget workbook")).unwrap();
    assert!(listed(&own).contains(&ids[1]));
    let parked = own
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == ids[1])
        .unwrap();
    assert_eq!(parked["lane"]["active"], json!(false));
    assert!(parked["lane"]["last_worked_days_ago"].as_i64().unwrap() >= 20);
}

#[test]
fn the_matcher_receives_a_bounded_trail_of_recent_windows_and_their_goals() {
    let (state, ids) = setup(&["Atlas launch alpha", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    feed(&mut i, &sample("Vega budget workbook"), base, 0, 60);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    let generic = sample("Research notes");
    feed(&mut i, &generic, base, 63, 123);
    let now = chrono::Utc::now().timestamp();
    let recent = i.goal_matching.recent_context(now, Instant::now());
    let trail = recent["trail"].as_array().unwrap();
    assert_eq!(
        trail.len(),
        1,
        "the current window is not part of the trail"
    );
    assert_eq!(trail[0]["app"], "Editor");
    assert_eq!(trail[0]["title"], "Vega budget workbook");
    assert_eq!(trail[0]["matched_goal_id"], json!(ids[1]));
    assert_eq!(recent["last_matched_goal_id"], json!(ids[1]));
    let job = claim(&mut i, base + Duration::from_secs(123))
        .unwrap()
        .unwrap();
    assert_eq!(job.recent["last_matched_goal_id"], json!(ids[1]));
    assert!(job.recent.to_string().contains("Vega budget workbook"));
    // Excluded or private windows never enter the trail; history deletion empties it.
    let mut private = sample("password manager vault");
    private.window_title = "Vault - 1Password".into();
    observe_inner_at(&mut i, &private, false, base + Duration::from_secs(126)).unwrap();
    let recent = i.goal_matching.recent_context(now, Instant::now());
    assert_eq!(recent["trail"].as_array().unwrap().len(), 2);
    assert!(!recent.to_string().contains("1Password"));
    for n in 0..20 {
        feed(
            &mut i,
            &sample(&format!("Window {n}")),
            base,
            200 + n * 6,
            203 + n * 6,
        );
    }
    assert!(i.goal_matching.trail.len() <= TRAIL_LIMIT);
    i.privacy_revision += 1;
    observe_inner_at(&mut i, &generic, true, base + Duration::from_secs(400)).unwrap();
    assert_eq!(
        i.goal_matching.trail.len(),
        1,
        "only the current window remains"
    );
}
#[test]
fn one_shared_word_or_a_domain_is_a_hint_that_neither_assigns_nor_blocks_the_matcher() {
    let (state, ids) = setup(&["Interview at Google", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let mut maps = sample("Google Maps");
    maps.process_name = "Google Chrome".into();
    maps.window_id = Some(5);
    maps.browser = crate::browser::context(
        &maps.process_name,
        &maps.window_title,
        Some("google.com".into()),
    );
    feed(&mut i, &maps, base, 0, 60);
    assert!(
        i.goal_matching.view.goal_id.is_none(),
        "a company name alone assigns nothing"
    );
    assert_eq!(attribution(&i, &maps).unwrap().confidence, 0.);
    assert!(due_inner(&i, base + AI_STABLE), "the matcher still runs");
    let goals = candidates(&i, &maps).unwrap();
    let interview = goals
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == ids[0])
        .unwrap();
    assert_eq!(interview["local_hint"], json!(true));
    let budget = goals
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == ids[1])
        .unwrap();
    assert!(budget["local_hint"].is_null());
    // A domain-only overlap is not even a hint, and two shared words still assign locally.
    let mut docs = sample("Quarterly report");
    docs.process_name = "Google Chrome".into();
    docs.browser = crate::browser::context(
        &docs.process_name,
        &docs.window_title,
        Some("docs.google.com".into()),
    );
    assert!(
        crate::relevance::hints(&[(ids[0], "Interview at Google".into(), false)], &docs).is_empty()
    );
    feed(&mut i, &sample("Vega budget workbook"), base, 63, 123);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
}
#[test]
fn hopping_between_windows_lets_the_main_one_become_stable_but_not_the_glanced_one() {
    let (state, _ids) = setup(&["Atlas launch alpha", "Atlas launch beta", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let main = sample("Atlas launch beta");
    let glance = sample("Research notes");
    // Twelve seconds in the main window, a three-second glance elsewhere, and again.
    feed(&mut i, &main, base, 0, 12);
    feed(&mut i, &glance, base, 15, 15);
    feed(&mut i, &main, base, 18, 30);
    feed(&mut i, &glance, base, 33, 33);
    feed(&mut i, &main, base, 36, 42);
    // Thirty seconds of actual presence have accumulated although no stint reached thirty.
    assert!(due_inner(&i, base + Duration::from_secs(42)));
    // The glanced window has only a few seconds of dwell and is never asked about.
    feed(&mut i, &glance, base, 45, 45);
    assert!(!due_inner(&i, base + Duration::from_secs(45)));
    feed(&mut i, &main, base, 48, 60);
    feed(&mut i, &glance, base, 63, 63);
    assert!(!due_inner(&i, base + Duration::from_secs(63)));
}

#[test]
fn a_dominant_learned_profile_assigns_locally_without_the_matcher() {
    let (state, ids) = setup(&["Apply to jobs on LinkedIn", "Create CV versions"]);
    let mut i = state.inner.lock().unwrap();
    let mut site = sample("Register – L'Oréal Groupe");
    site.process_name = "safari".into();
    site.window_id = Some(9);
    site.browser = crate::browser::context(
        "safari",
        &site.window_title,
        Some("careers-apply.loreal.com".into()),
    );
    // Six attributed minutes on the site teach the goal's profile.
    i.storage
        .learn_lane_profile(ids[0], &site, 360_000, 0.9)
        .unwrap();
    let base = Instant::now();
    feed(&mut i, &site, base, 0, 30);
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[0]));
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("learned_lane"));
    assert_eq!(i.goal_matching.metrics.reasoning_calls, 0);
    assert!(
        !due_inner(&i, base + Duration::from_secs(60)),
        "no cloud question is needed"
    );
    assert!(attribution(&i, &site).unwrap().confidence >= AUTO_ASSIGN_CONFIDENCE);
    // Split evidence hints the matcher instead of assigning.
    let (state, ids) = setup(&["Apply to jobs on LinkedIn", "Create CV versions"]);
    let mut i = state.inner.lock().unwrap();
    i.storage
        .learn_lane_profile(ids[0], &site, 360_000, 0.9)
        .unwrap();
    i.storage
        .learn_lane_profile(ids[1], &site, 240_000, 0.9)
        .unwrap();
    feed(&mut i, &site, base, 0, 30);
    assert!(i.goal_matching.view.goal_id.is_none());
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    let mut hinted: Vec<i64> = job
        .goals
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["learned_hint"] == json!(true))
        .map(|g| g["id"].as_i64().unwrap())
        .collect();
    hinted.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(hinted, expected);
}

#[test]
fn a_run_of_short_windows_with_enough_pending_time_is_asked_about_as_one_episode() {
    let (state, _ids) = setup(&["Atlas launch alpha", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let research = sample("Research notes");
    let draft = sample("Draft email");
    feed(&mut i, &research, base, 0, 180);
    feed(&mut i, &draft, base, 183, 195);
    // The newest window is twelve seconds old, yet the episode holds three unassigned minutes.
    let episode = i.goal_matching.episode(base + Duration::from_secs(195));
    assert_eq!(episode.windows, 2);
    assert!(episode.pending >= Duration::from_secs(190));
    assert!(due_inner(&i, base + Duration::from_secs(195)));
    let job = claim(&mut i, base + Duration::from_secs(195))
        .unwrap()
        .unwrap();
    assert_eq!(job.recent["episode"]["windows"], json!(2));
    assert!(job.recent["episode"]["pending_seconds"].as_u64().unwrap() >= 190);
    assert!(job.recent["trail"][0]["seconds"].as_i64().unwrap() >= 0);
    // Without an answer, the same episode is not asked about again before REASK.
    finish(
        &mut i,
        &job,
        Err("timeout".into()),
        base + Duration::from_secs(196),
    )
    .unwrap();
    let another = sample("Another window");
    feed(&mut i, &another, base, 199, 205);
    assert!(!due_inner(&i, base + Duration::from_secs(205)));
    // A gap longer than two minutes ends the episode; the next window starts a new one.
    feed(&mut i, &another, base, 330, 336);
    let fresh = i.goal_matching.episode(base + Duration::from_secs(336));
    assert_eq!(fresh.windows, 1);
    assert!(fresh.pending < Duration::from_secs(10));
}

#[test]
fn a_task_waiting_for_the_shared_lane_keeps_a_second_one_from_queueing() {
    let (state, _ids) = setup(&["Atlas launch alpha", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    feed(&mut i, &sample("Research notes"), base, 0, 45);
    feed(&mut i, &sample("Draft email"), base, 48, 60);
    assert!(due_inner(&i, base + Duration::from_secs(60)));
    i.goal_matching.lane_waiting = true;
    assert!(!due_inner(&i, base + Duration::from_secs(60)));
    i.goal_matching.lane_waiting = false;
    assert!(due_inner(&i, base + Duration::from_secs(60)));
}

#[test]
fn changing_text_in_the_same_window_is_asked_again_only_after_reask() {
    let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = sample("Atlas launch beta");
    feed(&mut i, &a, base, 0, 30);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: None,
            confidence: 0.4,
            reason: "Unclear".into(),
            same_task_trail: vec![],
        }),
        base + Duration::from_secs(40),
    )
    .unwrap();
    observe_text(&mut i, &a, "first draft");
    feed(&mut i, &a, base, 43, 60);
    assert!(
        !due_inner(&i, base + Duration::from_secs(60)),
        "cached null"
    );
    // New text invalidates the cache, but the window was asked about 30 s ago.
    observe_text(&mut i, &a, "second draft, more content");
    feed(&mut i, &a, base, 63, 90);
    assert!(i.goal_matching.cache.is_empty());
    assert!(!due_inner(&i, base + Duration::from_secs(90)));
    feed(&mut i, &a, base, 93, 123);
    assert!(due_inner(&i, base + Duration::from_secs(123)));
    // Another window is never held back by this one's question.
    let b = sample("Vega budget review");
    feed(&mut i, &b, base, 126, 156);
    assert!(
        due_inner(&i, base + Duration::from_secs(156))
            || i.goal_matching.view.goal_id == Some(ids[2])
    );
}

#[test]
fn an_unplanned_day_uses_recently_worked_lanes_as_its_plan() {
    // Today is empty: both lanes were created today, so both stand in for the plan.
    let (state, ids) = setup(&["Atlas launch alpha", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let date = i.storage.core_snapshot(None).unwrap().date;
    for id in &ids {
        i.storage.core_today(*id, false, &date).unwrap();
    }
    assert!(i.storage.core_snapshot(None).unwrap().today.is_empty());
    // Local keyword matching keeps working without a plan.
    assert_eq!(
        i.storage
            .matching_goals(&sample("Atlas launch alpha notes"))
            .unwrap(),
        vec![ids[0]]
    );
    let base = Instant::now();
    let a = sample("Research notes");
    feed(&mut i, &a, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    assert_eq!(job.goals[0]["lane"]["in_today"], json!(true));
    assert_eq!(job.goals[1]["lane"]["in_today"], json!(true));
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.82,
            reason: "Notes".into(),
            same_task_trail: vec![],
        }),
        base + AI_STABLE,
    )
    .unwrap();
    assert_eq!(
        i.goal_matching.view.goal_id,
        Some(ids[1]),
        "0.82 assigns a recently worked lane when no plan exists"
    );
}

#[test]
fn goals_outside_today_need_stronger_evidence_than_planned_ones() {
    let (state, ids) = setup(&["Atlas launch alpha", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let date = i.storage.core_snapshot(None).unwrap().date;
    i.storage.core_today(ids[1], false, &date).unwrap();
    let base = Instant::now();
    let a = sample("Research notes");
    feed(&mut i, &a, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    let answer = |id: i64, confidence: f64| MatchResult {
        goal_id: Some(id),
        confidence,
        reason: "Notes".into(),
        same_task_trail: vec![],
    };
    finish(&mut i, &job, Ok(answer(ids[1], 0.82)), base + AI_STABLE).unwrap();
    assert!(
        i.goal_matching.view.goal_id.is_none(),
        "0.82 is not enough outside Today"
    );
    assert!(
        i.goal_matching.pending_suggestion.is_some(),
        "it is offered instead"
    );
    i.goal_matching.clear();
    i.goal_matching.cache.clear();
    feed(&mut i, &a, base, 63, 123);
    let job = claim(&mut i, base + Duration::from_secs(150))
        .unwrap()
        .unwrap();
    finish(
        &mut i,
        &job,
        Ok(answer(ids[1], 0.86)),
        base + Duration::from_secs(150),
    )
    .unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    // The same 0.82 is enough for a goal planned for today.
    let (state, ids) = setup(&["Atlas launch alpha", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    feed(&mut i, &a, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    finish(&mut i, &job, Ok(answer(ids[1], 0.82)), base + AI_STABLE).unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
}
fn record_pending(i: &mut Inner, goal: i64, a: &ActivitySnapshot, from: i64, to: i64) {
    let wall = chrono::Utc::now();
    let unmatched = crate::relevance::ActivityMatch {
        goal_id: None,
        confidence: 0.,
        reason: "No verified relevance".into(),
    };
    let mut t = from;
    while t + 3 <= to {
        let interval = crate::history::UsageInterval {
            start: wall - chrono::Duration::seconds(t + 3),
            end: wall - chrono::Duration::seconds(t),
            activity: a.clone(),
            state: crate::tracking::ActivityState::Focused,
        };
        i.storage
            .record_interval_with_match(goal, &interval, Some(unmatched.clone()))
            .unwrap();
        i.storage
            .record_review_interval(&interval, &unmatched)
            .unwrap();
        t += 3;
    }
}
#[test]
fn a_confirmed_purchase_counts_its_earlier_stages_from_the_trail() {
    let (state, ids) = setup(&["Buy Dune tickets", "Vega budget"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let page = |title: &str, domain: &str| {
        let mut a = sample(title);
        a.process_name = "Google Chrome".into();
        a.window_id = Some(11);
        a.browser = crate::browser::context(&a.process_name, title, Some(domain.into()));
        a
    };
    let search = page("Showtimes - Cinema", "cinema.example");
    let seats = page("Choose seats - Cinema", "cinema.example");
    let pay = page("Checkout - Pay", "pay.example");
    // Searching, then seats, then payment; each stage was recorded without a goal.
    feed(&mut i, &search, base, 0, 60);
    record_pending(&mut i, ids[0], &search, 120, 180);
    feed(&mut i, &seats, base, 63, 120);
    record_pending(&mut i, ids[0], &seats, 60, 120);
    feed(&mut i, &pay, base, 123, 183);
    record_pending(&mut i, ids[0], &pay, 0, 60);
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 0);
    // The trail keeps wall-clock bounds; the test fed samples instantly, so set them.
    let now = chrono::Utc::now().timestamp();
    i.goal_matching.trail[0].at = now - 180;
    i.goal_matching.trail[0].until = now - 120;
    i.goal_matching.trail[1].at = now - 120;
    i.goal_matching.trail[1].until = now - 60;
    let job = claim(&mut i, base + Duration::from_secs(183))
        .unwrap()
        .unwrap();
    assert_eq!(job.trail.len(), 2);
    assert_eq!(job.recent["trail"][1]["title"], "Choose seats - Cinema");
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[0]),
            confidence: 0.9,
            reason: "Paying for the Dune tickets".into(),
            same_task_trail: vec![1],
        }),
        base + Duration::from_secs(183),
    )
    .unwrap();
    // Payment and seats count; the earlier search was not named and stays pending.
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 120);
    let pending: i64 = i
        .storage
        .connection
        .query_row(
            "SELECT COUNT(*) FROM review_intervals WHERE status='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 20);
    // An index outside the trail is rejected before anything is applied.
    assert!(parse(
        &json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"goal_id":ids[0],"confidence":0.9,"reason":"x","same_task_trail":[7]}).to_string()}}]}),
        &job.goals,
        job.trail.len()
    )
    .is_err());
}
#[test]
fn a_confident_refresh_corrects_an_automatic_assignment_once_without_double_counting() {
    let (state, ids) = setup(&["Vega budget review", "Vega forecast model"]);
    let mut i = state.inner.lock().unwrap();
    let base = Instant::now();
    let mut a = sample("Research notes");
    a.window_id = Some(7);
    // The AI assigns the first goal and the following minute is recorded under it.
    feed(&mut i, &a, base, 0, 60);
    let job = claim(&mut i, base + AI_STABLE).unwrap().unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[0]),
            confidence: 0.85,
            reason: "Budget notes".into(),
            same_task_trail: vec![],
        }),
        base + AI_STABLE,
    )
    .unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[0]));
    let wall = chrono::Utc::now();
    let local = attribution(&i, &a).unwrap();
    assert!(local.confidence >= crate::relevance::THRESHOLD);
    for n in 0..20 {
        let interval = crate::history::UsageInterval {
            start: wall - chrono::Duration::seconds(60 - n * 3),
            end: wall - chrono::Duration::seconds(57 - n * 3),
            activity: a.clone(),
            state: crate::tracking::ActivityState::Focused,
        };
        i.storage
            .record_interval_with_match(ids[0], &interval, Some(local.clone()))
            .unwrap();
        i.storage.record_review_interval(&interval, &local).unwrap();
    }
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 60);
    // Visible text changes, the match is carried and a refresh is requested.
    observe_text(&mut i, &a, "Budget sheet");
    observe_text(&mut i, &a, "Forecast model assumptions and scenarios");
    assert_eq!(i.goal_matching.view.source.as_deref(), Some("model_carry"));
    // The carried match keeps counting; the refresh waits for REASK since the first question.
    feed(&mut i, &a, base, 63, 93);
    assert!(!due_inner(&i, base + Duration::from_secs(93)));
    feed(&mut i, &a, base, 96, 123);
    let job = claim(&mut i, base + Duration::from_secs(123))
        .unwrap()
        .unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.92,
            reason: "The sheet is the forecast model".into(),
            same_task_trail: vec![],
        }),
        base + Duration::from_secs(123),
    )
    .unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.storage.relevant_seconds(ids[1], None).unwrap(), 60);
    assert_eq!(i.storage.relevant_seconds(ids[0], None).unwrap(), 0);
    let total: i64 = i
        .storage
        .connection
        .query_row("SELECT SUM(milliseconds) FROM usage_daily", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(total, 60_000, "moved, not duplicated");
    // A later answer naming the first goal again does not flip the visit back.
    i.goal_matching.cache.clear();
    i.goal_matching.view.source = Some("model_carry".into());
    let job = claim(&mut i, base + Duration::from_secs(215))
        .unwrap()
        .unwrap();
    finish(
        &mut i,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[0]),
            confidence: 0.95,
            reason: "Flip".into(),
            same_task_trail: vec![],
        }),
        base + Duration::from_secs(215),
    )
    .unwrap();
    assert_eq!(i.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(i.storage.relevant_seconds(ids[1], None).unwrap(), 60);
}
