use super::*;
use crate::{history::UsageInterval, storage::Storage, tracking::ActivityState};
use chrono::{Duration as CD, Timelike};

fn sample() -> ActivitySnapshot {
    ActivitySnapshot {
        process_name: "Google Chrome".into(),
        window_title: "Inbox – Gmail".into(),
        window_id: Some(7),
        browser: crate::browser::context(
            "Google Chrome",
            "Inbox – Gmail",
            Some("https://mail.google.com/mail/u/0/#inbox".into()),
        ),
        ..Default::default()
    }
}
fn interval(index: i64) -> UsageInterval {
    let noon = Local::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .earliest()
        .unwrap()
        .with_timezone(&Utc);
    UsageInterval {
        start: noon + CD::seconds(index * 3),
        end: noon + CD::seconds(index * 3 + 3),
        activity: sample(),
        state: ActivityState::Focused,
    }
}
fn fixtures(storage: &Storage, goal: Option<i64>) {
    for i in 0..60 {
        storage
            .observe_unplanned_interval(&interval(i), goal, &context_key(&sample()))
            .unwrap();
    }
}
fn ready() -> AppState {
    let s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let mut ai = s.ai_preferences().unwrap();
    ai.enabled = true;
    ai.share_goal_context = true;
    s.write_setting("ai_assistance_preferences", &ai).unwrap();
    let state = AppState::new(s).unwrap();
    {
        let mut i = state.inner.lock().unwrap();
        i.status.tracking = true;
        i.status.demo = false;
        i.status.mock_ai = false;
        i.status.nebius_configured = true;
        i.companion.view.preferences.screen_task_detection = true;
        i.buddy.foreground = Some(sample());
    }
    state
}
fn offer(inner: &mut Inner) -> String {
    let key = context_key(&sample());
    let id = "offer-1".to_string();
    inner.unplanned_work.pending = Some(Pending {
        view: Suggestion {
            id: id.clone(),
            title: "Handle unrelated emails".into(),
            seconds: 180,
            app: "chrome".into(),
            completed: true,
        },
        episodes: vec![Episode::all(&key)],
        key,
        day: Local::now().date_naive().to_string(),
        goals: goals(inner).unwrap(),
        receipt_key: None,
        consent: inner.last_consent_change,
        at: Instant::now(),
    });
    id
}
#[test]
fn confirmation_creates_a_completed_today_goal_without_focus_or_invented_time() {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    fixtures(&s, None);
    // A retry cannot inflate time.
    s.observe_unplanned_interval(&interval(0), None, &context_key(&sample()))
        .unwrap();
    let key = context_key(&sample());
    let day = Local::now().date_naive().to_string();
    assert_eq!(s.unplanned_milliseconds(&key, &day).unwrap(), 180000);
    assert!(s.core_snapshot(None).unwrap().goals.is_empty());
    let id = s
        .confirm_unplanned_work(
            &[Episode::all(&key)],
            &key,
            &day,
            "Handle unrelated emails",
            true,
            None,
        )
        .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    assert_eq!(snap.goals[0].id, id);
    assert_eq!(snap.goals[0].status, "completed");
    assert!(snap.today.contains(&id));
    assert!(snap.focused_goal_id.is_none());
    assert!(snap.timer.is_none());
    assert_eq!(snap.summary.goals[0].tracked_seconds, 180);
    assert_eq!(snap.summary.goals[0].relevant_seconds, 180);
    assert_eq!(
        snap.summary.completed_goals,
        vec!["Handle unrelated emails"]
    );
    assert_eq!(snap.hourly.iter().map(|h| h.seconds).sum::<u64>(), 180);
    assert!(s
        .confirm_unplanned_work(
            &[Episode::all(&key)],
            &key,
            &day,
            "Handle unrelated emails",
            true,
            None
        )
        .is_err());
    assert_eq!(s.core_snapshot(None).unwrap().goals.len(), 1);
}
#[test]
fn accepting_noticed_work_after_summary_updates_the_same_saved_measured_time() {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    fixtures(&s, None);
    for index in 0..60 {
        let mut i = interval(index);
        i.activity.timestamp = i.start.to_rfc3339();
        s.record_review_interval(
            &i,
            &crate::relevance::ActivityMatch {
                goal_id: None,
                confidence: 0.,
                reason: "Unknown".into(),
            },
        )
        .unwrap();
    }
    s.connection
        .execute("UPDATE review_periods SET analysis_done=1", [])
        .unwrap();
    s.finish_due_reviews(Utc::now() + CD::days(2)).unwrap();
    let goal = s
        .confirm_unplanned_work(
            &[Episode::all(&context_key(&sample()))],
            &context_key(&sample()),
            &Local::now().date_naive().to_string(),
            "Handle unrelated emails",
            false,
            None,
        )
        .unwrap();
    let overview = s.activity_review().unwrap();
    assert!(overview.summaries[0]
        .blocks
        .iter()
        .all(|b| b.status == "assigned" && b.goal_id == Some(goal)));
    assert_eq!(
        overview.summaries[0]
            .blocks
            .iter()
            .map(|b| b.seconds)
            .sum::<u64>(),
        180
    );
    assert_eq!(s.relevant_seconds(goal, None).unwrap(), 180);
}
#[test]
fn accepted_work_moves_intervals_and_step_totals_instead_of_double_counting() {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    s.core_create_goal(
        "Write the pitch deck",
        "pitch",
        &Local::now().date_naive().to_string(),
    )
    .unwrap();
    let old = s.ensure_today_focus().unwrap().unwrap();
    let mut plan = s.goal_plan(old).unwrap();
    plan.steps.push(crate::goals::Step {
        id: "s1".into(),
        text: "Draft slides".into(),
        done: false,
    });
    s.save_goal_plan("Write the pitch deck", plan).unwrap();
    for i in 0..60 {
        s.record_interval_with_match(
            old,
            &interval(i),
            Some(crate::relevance::ActivityMatch {
                goal_id: Some(old.to_string()),
                confidence: 1.0,
                reason: "Explicit focus".into(),
            }),
        )
        .unwrap();
        s.observe_unplanned_interval(&interval(i), Some(old), &context_key(&sample()))
            .unwrap();
    }
    let total: i64 = s
        .connection
        .query_row("SELECT SUM(milliseconds) FROM usage_daily", [], |r| {
            r.get(0)
        })
        .unwrap();
    let day = Local::now().date_naive().to_string();
    let new = s
        .confirm_unplanned_work(
            &[Episode::all(&context_key(&sample()))],
            &context_key(&sample()),
            &day,
            "Handle unrelated emails",
            true,
            None,
        )
        .unwrap();
    assert_eq!(
        s.connection
            .query_row("SELECT SUM(milliseconds) FROM usage_daily", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        total
    );
    assert_eq!(s.relevant_seconds(old, Some(&day)).unwrap(), 0);
    assert_eq!(s.relevant_seconds(new, Some(&day)).unwrap(), 180);
    assert_eq!(
        s.step_times(old, &day)
            .unwrap()
            .iter()
            .map(|s| s.seconds)
            .sum::<u64>(),
        0
    );
    assert_eq!(s.goal().unwrap().unwrap().id, old);
    assert_eq!(
        s.connection
            .query_row("SELECT COUNT(*) FROM usage_intervals", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        60
    );
}
#[test]
fn interval_collection_skips_idle_media_midnight_and_unobserved_gaps() {
    let s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let mut a = interval(0);
    a.state = ActivityState::Paused;
    s.observe_unplanned_interval(&a, None, &context_key(&sample()))
        .unwrap();
    a = interval(0);
    a.activity.media_playing = true;
    s.observe_unplanned_interval(&a, None, &context_key(&sample()))
        .unwrap();
    a = interval(0);
    a.end = a.start + CD::seconds(60);
    s.observe_unplanned_interval(&a, None, &context_key(&sample()))
        .unwrap();
    assert_eq!(
        s.unplanned_milliseconds(
            &context_key(&sample()),
            &Local::now().date_naive().to_string()
        )
        .unwrap(),
        0
    );
    let mut tracker = crate::insights::UsageTracker::default();
    let now = Instant::now();
    let wall = Utc::now();
    assert!(tracker
        .observe_at(0, &sample(), true, ActivityState::Focused, now, wall)
        .is_none());
    assert!(tracker
        .observe_at(
            0,
            &sample(),
            true,
            ActivityState::Focused,
            now + Duration::from_secs(60),
            wall + CD::seconds(60)
        )
        .is_none());
    assert!(tracker
        .observe_at(
            0,
            &sample(),
            true,
            ActivityState::Paused,
            now + Duration::from_secs(63),
            wall + CD::seconds(63)
        )
        .is_none());
}
#[test]
fn opt_out_sensitive_apps_timer_and_stale_context_prevent_collection_or_disclosure() {
    let state = ready();
    let mut inner = state.inner.lock().unwrap();
    assert!(active(&inner, &sample(), true));
    inner.companion.view.preferences.screen_task_detection = false;
    observe(&mut inner, &sample(), true, None).unwrap();
    assert!(inner.unplanned_work.sample.is_none());
    inner.companion.view.preferences.screen_task_detection = true;
    let secret = ActivitySnapshot {
        process_name: "1password".into(),
        ..sample()
    };
    assert!(!active(&inner, &secret, true));
    let secret_title = ActivitySnapshot {
        window_title: "api_key=fictional-secret-canary".into(),
        ..sample()
    };
    assert!(!active(&inner, &secret_title, true));
    let g = goals(&inner).unwrap();
    inner.unplanned_work.sample = Some(sample());
    let generation = inner.unplanned_work.generation;
    assert!(current(&inner, &sample(), &g, 0, generation).unwrap());
    inner.unplanned_work.reset_sample();
    inner.unplanned_work.sample = Some(sample());
    assert!(
        !current(&inner, &sample(), &g, 0, generation).unwrap(),
        "Leaving and returning cannot revive an in-flight response"
    );
    inner
        .storage
        .core_create_goal("Pitch", "pitch", &Local::now().date_naive().to_string())
        .unwrap();
    let goal = inner.storage.ensure_today_focus().unwrap().unwrap();
    inner.storage.core_set_timer(Some(goal)).unwrap();
    assert!(!active(&inner, &sample(), true));
}
#[test]
fn offers_survive_switching_to_sumi_but_consent_changes_and_history_deletion_invalidate_them() {
    let state = ready();
    let mut i = state.inner.lock().unwrap();
    fixtures(&i.storage, None);
    let id = offer(&mut i);
    observe(
        &mut i,
        &ActivitySnapshot {
            process_name: "desktop-buddy".into(),
            ..sample()
        },
        false,
        None,
    )
    .unwrap();
    assert!(pending(&mut i).unwrap().is_some());
    i.mark_consent_change();
    assert!(respond(&mut i, &id, "add", None).is_err());
    let id = offer(&mut i);
    i.storage.purge_history(None).unwrap();
    assert!(respond(&mut i, &id, "add", None).is_err());
}
#[test]
fn chat_opens_an_editable_topic_without_technical_observation_details_or_new_goals() {
    let state = ready();
    let mut inner = state.inner.lock().unwrap();
    fixtures(&inner.storage, None);
    let id = offer(&mut inner);
    let title = inner
        .unplanned_work
        .pending
        .as_ref()
        .unwrap()
        .view
        .title
        .clone();
    assert!(respond(&mut inner, &id, "chat", None).unwrap().is_none());
    assert!(inner.companion.view.chat_open);
    assert_eq!(
        inner.companion.view.seed,
        crate::companion::topic_chat_draft(&title)
    );
    assert!(inner.storage.chat_history().unwrap().is_empty());
    assert!(inner.storage.core_snapshot(None).unwrap().goals.is_empty());
}

#[test]
fn dismiss_and_snooze_never_create_goals_and_rejections_follow_history_retention() {
    let state = ready();
    let mut i = state.inner.lock().unwrap();
    fixtures(&i.storage, None);
    let id = offer(&mut i);
    assert!(respond(&mut i, &id, "not_now", None).unwrap().is_none());
    assert!(i.unplanned_work.snoozed_until.unwrap() > Instant::now());
    let id = offer(&mut i);
    respond(&mut i, &id, "dismiss", None).unwrap();
    let key = context_key(&sample());
    let day = Local::now().date_naive().to_string();
    assert!(i.storage.unplanned_decided(&key, &day).unwrap());
    assert!(i.storage.core_snapshot(None).unwrap().goals.is_empty());
    i.storage
        .purge_history(Some(&(Utc::now() - CD::days(7)).to_rfc3339()))
        .unwrap();
    assert!(i.storage.unplanned_decided(&key, &day).unwrap());
    i.storage.purge_history(None).unwrap();
    assert!(!i.storage.unplanned_decided(&key, &day).unwrap());
}
#[test]
fn response_validation_requires_a_new_unambiguous_task_and_valid_candidate_ids() {
    let candidates = json!([{"id":7,"title":"Prepare a deck"}]);
    let envelope = |v| crate::accuracy_check::envelope(v);
    assert!(parse(
        &envelope(json!({"title":"Handle emails","existing_goal_id":7,"confidence":0.9})),
        &candidates,
        0
    )
    .is_err());
    assert!(parse(
        &envelope(json!({"title":null,"existing_goal_id":77,"confidence":0.9})),
        &candidates,
        0
    )
    .is_err());
    assert!(parse(
        &envelope(
            json!({"title":"Write to person@example.com","existing_goal_id":null,"confidence":0.9})
        ),
        &candidates,
        0
    )
    .is_err());
    assert!(parse(
        &envelope(json!({"title":null,"existing_goal_id":7,"confidence":0.9})),
        &candidates,
        0
    )
    .is_ok());
}
#[tokio::test]
async fn classifier_uses_minimized_redacted_context_once_without_credentials_or_raw_urls() {
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|r:&wiremock::Request|{
        let body:Value=serde_json::from_slice(&r.body).unwrap();
        let context:Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(context["activity"]["domain"],"mail.google.com");
        assert!(context["activity"]["visible_text"].as_str().unwrap().chars().count()<=crate::companion::screen_context::EXCERPT_CHARS);
        assert!(!body.to_string().contains("fictional-secret-canary"));
        assert!(!body.to_string().contains("isolated-test-key"));
        assert!(!body.to_string().contains("/mail/u/0"));
        ResponseTemplate::new(200).set_body_json(crate::accuracy_check::envelope(json!({"title":"Handle unrelated emails","existing_goal_id":null,"confidence":0.94})))
    }).expect(1).mount(&server).await;
    let client = reqwest::Client::new();
    let service = Service {
        client: &client,
        endpoint: server.uri(),
        key: "isolated-test-key".into(),
        model: "fixture".into(),
    };
    let text = format!(
        "api_key=fictional-secret-canary\nInbox\n{}",
        "Email replies ".repeat(100)
    );
    let result = classify(
        &service,
        &sample(),
        &text,
        &json!([]),
        180,
        &Value::Null,
        &Value::Null,
        &|| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(result.title.as_deref(), Some("Handle unrelated emails"));
}

#[test]
fn returning_to_the_same_window_never_relabels_an_earlier_session() {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    for n in 0..60 {
        s.observe_unplanned_interval(&interval(n), None, "morning-session")
            .unwrap();
    }
    for n in 100..160 {
        s.observe_unplanned_interval(&interval(n), None, "afternoon-session")
            .unwrap();
    }
    let day = Local::now().date_naive().to_string();
    let id = s
        .confirm_unplanned_work(
            &[Episode::all("afternoon-session")],
            "afternoon-session",
            &day,
            "Handle unrelated emails",
            true,
            None,
        )
        .unwrap();
    assert_eq!(s.relevant_seconds(id, Some(&day)).unwrap(), 180);
    assert_eq!(
        s.unplanned_milliseconds("morning-session", &day).unwrap(),
        180000
    );
    assert!(s.unplanned_decided(&context_key(&sample()), &day).unwrap());
}

#[test]
fn ongoing_work_is_added_open_and_acceptance_does_not_double_count_the_observation() {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    fixtures(&s, None);
    let key = context_key(&sample());
    let day = Local::now().date_naive().to_string();
    let observed = s
        .record_work_event(
            &format!("noticed:{key}"),
            None,
            None,
            "Prepare interview follow-ups",
            "Browser",
            crate::work_journal::Outcome::Observed,
        )
        .unwrap();
    let id = s
        .confirm_unplanned_work(
            &[Episode::all(&key)],
            &key,
            &day,
            "Prepare interview follow-ups",
            false,
            None,
        )
        .unwrap();
    let snap = s.core_snapshot(None).unwrap();
    assert_eq!(snap.goals[0].status, "open");
    assert!(snap.today.contains(&id));
    assert!(snap.summary.completed_goals.is_empty());
    let events = s.work_journal(&day).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, "planned");
    let source: i64 = s
        .connection
        .query_row(
            "SELECT source_event_id FROM work_events WHERE goal_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(source, observed);
}

#[test]
fn observation_excerpt_preserves_late_receipts_and_bounded_untrusted_visual_context() {
    let native = format!(
        "{}Message sent successfully. Undo.",
        "Public inbox row. ".repeat(80)
    );
    let text = format!(
        "{native}\n[Visual context — unverified]\nFollow-up composer\npassword=do-not-disclose"
    );
    let excerpt = observation_excerpt(&sample(), &text).unwrap();
    assert!(excerpt.contains("Message sent successfully. Undo."));
    assert!(excerpt.contains("[Visual context — unverified]\nFollow-up composer"));
    assert!(!excerpt.contains("do-not-disclose"));
    assert!(excerpt.chars().count() <= crate::companion::screen_context::EXCERPT_CHARS);
    let mut private = sample();
    private.window_title = "Private browsing".into();
    assert!(observation_excerpt(&private, &text).is_none());
}

#[test]
fn unlisted_work_checks_continue_after_hours_and_quiet_discards_pending_offers() {
    let state = ready();
    let mut inner = state.inner.lock().unwrap();
    let now = Instant::now();
    let current_minute = Local::now().hour() * 60 + Local::now().minute();
    inner.tracking_settings.working_start_minute = (current_minute + 60) % 1440;
    inner.tracking_settings.working_end_minute = (current_minute + 120) % 1440;
    inner.unplanned_work.sample = Some(sample());
    inner.unplanned_work.stable_since = Some(now - STABLE);
    inner.unplanned_work.last_seen = Some(now);
    assert!(due_inner(&inner, now));
    // The same context is asked about again only after REASK, however often its text changes.
    inner.unplanned_work.attempted_at =
        Some((context_key(&sample()), now - Duration::from_secs(60)));
    assert!(!due_inner(&inner, now));
    inner.unplanned_work.attempted_at = Some((context_key(&sample()), now - REASK));
    assert!(due_inner(&inner, now));
    let mut other = sample();
    other.window_title = "Another window".into();
    inner.unplanned_work.attempted_at = Some((context_key(&other), now));
    assert!(due_inner(&inner, now), "another context is not debounced");
    inner.unplanned_work.attempted_at = None;
    // Work that goal matching already assigned is never offered as new work.
    inner.goal_matching.view.goal_id = Some(1);
    assert!(!due_inner(&inner, now));
    inner.goal_matching.view.goal_id = None;
    fixtures(&inner.storage, None);
    offer(&mut inner);
    assert!(pending(&mut inner).unwrap().is_some());
    inner.status.dnd = true;
    crate::companion::enforce_quiet(&mut inner);
    assert!(pending(&mut inner).unwrap().is_none());
    assert!(!due_inner(&inner, now));
    inner.status.dnd = false;
    assert!(pending(&mut inner).unwrap().is_none());
    assert_eq!(
        inner
            .storage
            .unplanned_milliseconds(
                &context_key(&sample()),
                &Local::now().date_naive().to_string()
            )
            .unwrap(),
        180_000
    );
}
#[test]
fn time_is_filed_per_context_regardless_of_review_segments_or_gaps() {
    let state = ready();
    let mut inner = state.inner.lock().unwrap();
    let base = Instant::now();
    observe_at(&mut inner, &sample(), true, None, base).unwrap();
    assert_eq!(
        inner.unplanned_work.session_key.as_deref(),
        Some(context_key(&sample()).as_str())
    );
    observe_at(
        &mut inner,
        &sample(),
        true,
        None,
        base + Duration::from_secs(360),
    )
    .unwrap();
    assert_eq!(
        inner.unplanned_work.session_key.as_deref(),
        Some(context_key(&sample()).as_str())
    );
    for i in 0..120 {
        inner
            .storage
            .observe_unplanned_interval(&interval(i), None, &context_key(&sample()))
            .unwrap();
    }
    let day = Local::now().date_naive().to_string();
    assert_eq!(
        inner
            .storage
            .unplanned_milliseconds(&context_key(&sample()), &day)
            .unwrap(),
        360_000
    );
}
fn page(title: &str, domain: &str) -> ActivitySnapshot {
    let mut a = sample();
    a.window_title = title.into();
    a.browser = crate::browser::context(&a.process_name, title, Some(format!("https://{domain}/")));
    a
}
fn file(storage: &Storage, a: &ActivitySnapshot, from: i64, to: i64) {
    for i in from..to {
        let mut interval = interval(i);
        interval.activity = a.clone();
        storage
            .observe_unplanned_interval(&interval, None, &context_key(a))
            .unwrap();
    }
}
#[test]
fn acceptance_moves_the_presence_and_the_named_episodes_only() {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let day = Local::now().date_naive().to_string();
    let noon = interval(0).start;
    let posting = sample();
    let search = page("Blockchain.com careers - Google Search", "www.google.com");
    let video = page("Trending - YouTube", "www.youtube.com");
    // An earlier visit to the posting, a search, a video, then the current presence.
    file(&s, &posting, -200, -160);
    file(&s, &search, -100, -60);
    file(&s, &video, -50, -30);
    file(&s, &posting, 0, 60);
    let episodes = vec![
        Episode {
            key: context_key(&posting),
            since: Some(noon - CD::seconds(2)),
            until: None,
        },
        Episode {
            key: context_key(&search),
            since: Some(noon - CD::seconds(302)),
            until: Some(noon - CD::seconds(178)),
        },
    ];
    let goal = s
        .confirm_unplanned_work(
            &episodes,
            &context_key(&posting),
            &day,
            "Prepare for the Blockchain.com interview",
            false,
            None,
        )
        .unwrap();
    // 180 s of the presence and 120 s of the search moved; nothing else did.
    assert_eq!(s.relevant_seconds(goal, Some(&day)).unwrap(), 300);
    assert_eq!(
        s.unplanned_milliseconds(&context_key(&posting), &day)
            .unwrap(),
        120_000,
        "the earlier visit to the same page stays unassigned"
    );
    assert_eq!(
        s.unplanned_milliseconds(&context_key(&search), &day)
            .unwrap(),
        0
    );
    assert_eq!(
        s.unplanned_milliseconds(&context_key(&video), &day)
            .unwrap(),
        60_000
    );
    assert!(s.unplanned_decided(&context_key(&posting), &day).unwrap());
}
#[test]
fn the_classifier_may_only_name_trail_entries_it_was_shown() {
    let answer = |trail: Value| {
        crate::accuracy_check::envelope(json!({
            "title":"Prepare for the interview","existing_goal_id":null,"confidence":0.9,"receipt":null,"same_task_trail":trail
        }))
    };
    assert_eq!(
        parse(&answer(json!([1])), &json!([]), 2)
            .unwrap()
            .same_task_trail,
        vec![1]
    );
    assert!(parse(&answer(json!([2])), &json!([]), 2).is_err());
    assert!(parse(&answer(json!([])), &json!([]), 0)
        .unwrap()
        .same_task_trail
        .is_empty());
}
