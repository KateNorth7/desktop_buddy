use super::*;
use cascade_tests::{feed, sample, setup};

fn browser(domain: Option<&str>) -> ActivitySnapshot {
    let mut a = sample("Orion launch - Browser");
    a.process_name = "chrome.exe".into();
    a.window_id = Some(101);
    a.browser =
        crate::browser::context(&a.process_name, &a.window_title, domain.map(str::to_string));
    a
}

#[test]
fn short_metadata_gap_pauses_recognition_without_assigning_unknown_time() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut inner = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = browser(Some("example.com"));
    let unknown = browser(None);
    feed(&mut inner, &a, base, 0, 27);
    observe_inner_at(&mut inner, &unknown, true, base + Duration::from_secs(30)).unwrap();
    assert!(inner.goal_matching.view.goal_id.is_none());
    assert_eq!(attribution(&inner, &unknown).unwrap().confidence, 0.);
    assert_eq!(
        inner
            .goal_matching
            .current_sample()
            .unwrap()
            .browser
            .as_ref()
            .unwrap()
            .domain,
        None
    );
    feed(&mut inner, &a, base, 33, 36);
    // Only 27s before the gap + 3s after it count toward the unchanged 30s gate.
    assert_eq!(inner.goal_matching.view.goal_id, Some(ids[0]));
}

#[test]
fn metadata_gap_limit_is_inclusive_and_starts_at_the_first_missing_sample() {
    for recovery in [36, 37] {
        let (state, ids) = setup(&["Orion launch", "Vega budget"]);
        let mut inner = state.inner.lock().unwrap();
        let base = Instant::now();
        let a = browser(Some("example.com"));
        feed(&mut inner, &a, base, 0, 27);
        feed(&mut inner, &browser(None), base, 30, 33);
        feed(&mut inner, &a, base, recovery, recovery + 3);
        assert_eq!(
            inner.goal_matching.view.goal_id,
            (recovery == 36).then_some(ids[0])
        );
        feed(&mut inner, &a, base, recovery + 6, recovery + 30);
        assert_eq!(inner.goal_matching.view.goal_id, Some(ids[0]));
    }
}

#[test]
fn repeated_short_gaps_exclude_unknown_time_without_starving_local_recognition() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut inner = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = browser(Some("example.com"));
    for t in (0..=60).step_by(3) {
        let current = if t % 15 == 12 {
            browser(None)
        } else {
            a.clone()
        };
        observe_inner_at(&mut inner, &current, true, base + Duration::from_secs(t)).unwrap();
        if current.browser.as_ref().unwrap().domain.is_none() {
            assert!(inner.goal_matching.view.goal_id.is_none());
            assert!(attribution(&inner, &current).unwrap().goal_id.is_none());
        }
    }
    assert_eq!(inner.goal_matching.view.goal_id, Some(ids[0]));
    assert_eq!(inner.goal_matching.metrics.reasoning_calls, 0);
}

#[test]
fn confirmed_context_changes_and_missing_window_identity_restart_the_clock() {
    for gap in [false, true] {
        for change in 0..7 {
            let (state, _) = setup(&["Orion launch", "Vega budget"]);
            let mut inner = state.inner.lock().unwrap();
            let base = Instant::now();
            let a = browser(Some("example.com"));
            feed(&mut inner, &a, base, 0, 27);
            if gap {
                observe_inner_at(
                    &mut inner,
                    &browser(None),
                    true,
                    base + Duration::from_secs(30),
                )
                .unwrap();
            }
            let mut b = a.clone();
            match change {
                0 => b.browser.as_mut().unwrap().domain = Some("other.example.com".into()),
                1 => b.window_id = Some(102),
                2 => b.window_title = "Orion launch - Another tab".into(),
                3 => b.browser.as_mut().unwrap().page_title = "Another page".into(),
                4 => b.browser.as_mut().unwrap().browser = "edge".into(),
                5 => b.process_name = "msedge.exe".into(),
                _ => b.window_id = None,
            }
            observe_inner_at(&mut inner, &b, true, base + Duration::from_secs(33)).unwrap();
            assert_eq!(
                inner.goal_matching.stable_since,
                Some(base + Duration::from_secs(33)),
                "gap={gap}, change={change}"
            );
            assert!(inner.goal_matching.view.goal_id.is_none());
        }
    }
    let mut runtime = Runtime::default();
    let base = Instant::now();
    let mut a = browser(Some("example.com"));
    a.window_id = None;
    runtime.observe_at(&a, true, base);
    let mut unknown = a.clone();
    unknown.browser = crate::browser::context(&a.process_name, &a.window_title, None);
    runtime.observe_at(&unknown, true, base + Duration::from_secs(3));
    runtime.observe_at(&a, true, base + Duration::from_secs(6));
    assert_eq!(runtime.stable_since, Some(base + Duration::from_secs(6)));
}

#[test]
fn prolonged_missing_metadata_stays_uncertain_even_with_explicit_focus() {
    for explicit in [false, true] {
        let (state, ids) = setup(&["Orion launch", "Vega budget"]);
        let mut inner = state.inner.lock().unwrap();
        let base = Instant::now();
        let a = browser(Some("example.com"));
        feed(&mut inner, &a, base, 0, 30);
        if explicit {
            inner.storage.mark_explicit_focus(ids[0]).unwrap();
        }
        let unknown = browser(None);
        feed(&mut inner, &unknown, base, 33, 123);
        assert!(inner.goal_matching.view.goal_id.is_none());
        assert!(inner.goal_matching.stable_since.is_none());
        assert!(inner.goal_matching.metadata_gap.is_none());
        // Nothing local, cached or focused applies to a title-only window; only the AI
        // may be asked about it, and not while an explicit focus stands.
        assert_eq!(
            due_inner(&inner, base + Duration::from_secs(123)),
            !explicit
        );
        assert!(attribution(&inner, &unknown).unwrap().goal_id.is_none());
        observe_text(
            &mut inner,
            &unknown,
            "Old page content must not be restored",
        );
        accept_context_match(&mut inner, &unknown, ids[1], 0.99).unwrap();
        assert!(inner.goal_matching.text_fingerprint.is_none());
        assert!(inner.goal_matching.view.goal_id.is_none());
        observe_inner_at(&mut inner, &a, true, base + Duration::from_secs(126)).unwrap();
        assert_eq!(
            inner.goal_matching.stable_since,
            Some(base + Duration::from_secs(126))
        );
    }
}

#[test]
fn a_metadata_gap_invalidates_ai_results_suggestions_and_page_evidence_epochs() {
    for cache_result in [false, true] {
        for confidence in [0.7, 0.95] {
            let (state, ids) = setup(&["Atlas launch alpha", "Atlas launch beta"]);
            let mut inner = state.inner.lock().unwrap();
            let base = Instant::now();
            let mut a = browser(Some("example.com"));
            a.window_title = "Atlas launch".into();
            a.browser.as_mut().unwrap().page_title = a.window_title.clone();
            feed(&mut inner, &a, base, 0, 60);
            let job = claim(&mut inner, base + AI_STABLE).unwrap().unwrap();
            let result = || {
                Ok(MatchResult {
                    goal_id: Some(ids[1]),
                    confidence,
                    reason: "Old page match".into(),
                    same_task_trail: vec![],
                })
            };
            if cache_result {
                finish(&mut inner, &job, result(), base + AI_STABLE).unwrap();
            }
            let epoch = inner.companion.screen_context_epoch;
            let mut unknown = a.clone();
            unknown.browser = crate::browser::context(&a.process_name, &a.window_title, None);
            observe_inner_at(&mut inner, &unknown, true, base + Duration::from_secs(63)).unwrap();
            assert!(inner.goal_matching.cache.is_empty());
            assert!(inner.goal_matching.pending_suggestion.is_none());
            assert!(!context_current(
                &inner,
                job.revision,
                job.context_revision,
                &job.goals,
                &job.sample
            ));
            observe_inner_at(&mut inner, &a, true, base + Duration::from_secs(66)).unwrap();
            assert!(inner.companion.screen_context_epoch >= epoch + 2);
            finish(&mut inner, &job, result(), base + Duration::from_secs(66)).unwrap();
            assert!(inner.goal_matching.view.goal_id.is_none());
            assert!(inner.goal_matching.cache.is_empty());
            assert!(inner.goal_matching.pending_suggestion.is_none());
            // The gap invalidated the old answer, so fresh evidence is asked about at once.
            feed(&mut inner, &a, base, 69, 75);
            assert!(due_inner(&inner, base + Duration::from_secs(75)));
        }
    }
}

#[test]
fn disallowed_observation_discards_any_paused_recognition_time() {
    let (state, _) = setup(&["Orion launch", "Vega budget"]);
    let mut inner = state.inner.lock().unwrap();
    let base = Instant::now();
    let a = browser(Some("example.com"));
    feed(&mut inner, &a, base, 0, 27);
    observe_inner_at(
        &mut inner,
        &browser(None),
        true,
        base + Duration::from_secs(30),
    )
    .unwrap();
    observe_inner_at(&mut inner, &a, false, base + Duration::from_secs(31)).unwrap();
    observe_inner_at(&mut inner, &a, true, base + Duration::from_secs(33)).unwrap();
    assert_eq!(
        inner.goal_matching.stable_since,
        Some(base + Duration::from_secs(33))
    );
}

#[test]
fn title_only_browser_windows_match_through_the_ai_alone_and_are_never_reused() {
    let (state, ids) = setup(&["Orion launch", "Vega budget"]);
    let mut inner = state.inner.lock().unwrap();
    let base = Instant::now();
    // The title names the first goal, but keywords never apply without a domain.
    let unknown = browser(None);
    feed(&mut inner, &unknown, base, 0, 27);
    assert!(inner.goal_matching.view.goal_id.is_none());
    assert!(!due_inner(&inner, base + Duration::from_secs(27)));
    observe_inner_at(&mut inner, &unknown, true, base + AI_STABLE).unwrap();
    assert!(due_inner(&inner, base + AI_STABLE));
    let job = claim(&mut inner, base + AI_STABLE).unwrap().unwrap();
    assert!(job.recent["trail"].as_array().unwrap().is_empty());
    finish(
        &mut inner,
        &job,
        Ok(MatchResult {
            goal_id: Some(ids[1]),
            confidence: 0.9,
            reason: "The page text is the budget sheet".into(),
            same_task_trail: vec![],
        }),
        base + AI_STABLE,
    )
    .unwrap();
    assert_eq!(inner.goal_matching.view.goal_id, Some(ids[1]));
    assert_eq!(
        inner.goal_matching.view.source.as_deref(),
        Some("model_unverified")
    );
    assert!(inner.goal_matching.cache.is_empty());
    assert_eq!(inner.storage.goal().unwrap().unwrap().id, ids[1]);
    feed(&mut inner, &unknown, base, 33, 120);
    assert_eq!(inner.goal_matching.view.goal_id, Some(ids[1]));
    let matched = attribution(&inner, &unknown).unwrap();
    assert_eq!(
        matched.goal_id.as_deref(),
        Some(ids[1].to_string().as_str())
    );
    assert!(matched.confidence >= crate::relevance::THRESHOLD);
    assert!(!due_inner(&inner, base + Duration::from_secs(200)));
    // Another title in the same window inherits nothing and waits its own minute.
    let mut other = unknown.clone();
    other.window_title = "Another tab - Browser".into();
    other.browser = crate::browser::context(&other.process_name, &other.window_title, None);
    feed(&mut inner, &other, base, 123, 150);
    assert!(inner.goal_matching.view.goal_id.is_none());
    assert_eq!(attribution(&inner, &other).unwrap().confidence, 0.);
    assert!(!due_inner(&inner, base + Duration::from_secs(150)));
    observe_inner_at(&mut inner, &other, true, base + Duration::from_secs(153)).unwrap();
    assert!(due_inner(&inner, base + Duration::from_secs(153)));
    // A null answer is not asked again for the same visit.
    let job = claim(&mut inner, base + Duration::from_secs(153))
        .unwrap()
        .unwrap();
    finish(
        &mut inner,
        &job,
        Ok(MatchResult {
            goal_id: None,
            confidence: 0.1,
            reason: "Unrelated".into(),
            same_task_trail: vec![],
        }),
        base + Duration::from_secs(153),
    )
    .unwrap();
    feed(&mut inner, &other, base, 156, 400);
    assert!(inner.goal_matching.view.goal_id.is_none());
    assert!(!due_inner(&inner, base + Duration::from_secs(400)));
    assert_eq!(inner.goal_matching.metrics.reasoning_calls, 2);
}
