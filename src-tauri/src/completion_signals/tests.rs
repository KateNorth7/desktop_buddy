use super::*;
use crate::{commands::AppState, core::GoalDraft, storage::Storage};

fn setup(titles: &[&str]) -> (AppState, Vec<i64>) {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    for (i, title) in titles.iter().enumerate() {
        s.core_add_goals(
            vec![GoalDraft {
                title: (*title).into(),
                area: "Work".into(),
                steps: vec![],
            }],
            &format!("signal-{i}"),
        )
        .unwrap();
    }
    let snap = s.core_snapshot(None).unwrap();
    let ids = titles
        .iter()
        .map(|t| snap.goals.iter().find(|g| &g.title == t).unwrap().id)
        .collect();
    (AppState::new(s).unwrap(), ids)
}
fn window(app: &str, title: &str, domain: Option<&str>) -> ActivitySnapshot {
    let mut a = ActivitySnapshot {
        process_name: app.into(),
        window_title: title.into(),
        idle_seconds: 1,
        window_id: Some(if domain.is_some() { 7 } else { 8 }),
        ..Default::default()
    };
    if let Some(domain) = domain {
        a.browser = crate::browser::context(app, title, Some(domain.into()));
    }
    a
}
fn status(inner: &Inner, id: i64) -> String {
    inner
        .storage
        .core_work_snapshot()
        .unwrap()
        .goals
        .into_iter()
        .find(|g| g.id == id)
        .map(|g| g.status)
        .unwrap_or_default()
}
fn outcomes(inner: &Inner) -> Vec<String> {
    let mut q = inner
        .storage
        .connection
        .prepare("SELECT outcome FROM tracking_attempts WHERE stage='outcome_check' ORDER BY id")
        .unwrap();
    q.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn run(
    inner: &mut Inner,
    a: &ActivitySnapshot,
    attributed: Option<i64>,
    base: Instant,
    from: u64,
    to: u64,
) {
    for t in (from..=to).step_by(3) {
        observe(inner, a, true, attributed, base + Duration::from_secs(t)).unwrap();
    }
}

#[test]
fn only_explicit_phrasing_names_a_simple_goal() {
    assert_eq!(classify("Зайти на созвон", ""), GoalKind::JoinCall);
    assert_eq!(
        classify("Join the Google Meet call", ""),
        GoalKind::JoinCall
    );
    assert_eq!(
        classify("Подключиться к встрече с командой", ""),
        GoalKind::JoinCall
    );
    assert_eq!(classify("Подготовиться к созвону", ""), GoalKind::Other);
    assert_eq!(
        classify("Schedule a meeting with Anna", ""),
        GoalKind::Other
    );
    assert_eq!(
        classify("Зайти на сайт payoneer", ""),
        GoalKind::VisitSite("payoneer".into())
    );
    assert_eq!(
        classify("Visit linear.app", ""),
        GoalKind::VisitSite("linear.app".into())
    );
    assert_eq!(
        classify("Open www.example.com pricing page", ""),
        GoalKind::VisitSite("example.com".into())
    );
    assert_eq!(classify("Написать письмо HR", ""), GoalKind::SendEmail);
    assert_eq!(
        classify("Send the follow-up email", ""),
        GoalKind::SendEmail
    );
    assert_eq!(
        classify(
            "Указать доступность для интервью Payoneer (Finance Transformation Specialist)",
            ""
        ),
        GoalKind::Other
    );
    assert_eq!(
        classify("Export updates and fix keyboard input issue in Life OS", ""),
        GoalKind::Other
    );
}

#[test]
fn meeting_windows_are_recognised_but_chat_windows_are_not() {
    assert!(in_meeting(&window(
        "Google Chrome",
        "Meet - abc-defg-hij",
        Some("meet.google.com")
    )));
    assert!(in_meeting(&window("zoom.us", "Zoom Meeting", None)));
    assert!(in_meeting(&window("Slack", "Huddle with Design", None)));
    assert!(in_meeting(&window("Telegram", "Звонок — Анна", None)));
    assert!(in_meeting(&window(
        "Microsoft Teams",
        "Weekly sync | Meeting",
        None
    )));
    assert!(!in_meeting(&window("zoom.us", "Zoom Workplace", None)));
    assert!(!in_meeting(&window("Telegram", "Telegram", None)));
    assert!(!in_meeting(&window(
        "Google Chrome",
        "Inbox - Gmail",
        Some("mail.google.com")
    )));
}

#[test]
fn a_join_call_goal_closes_after_a_minute_in_the_meeting_with_a_notice() {
    let (state, ids) = setup(&["Зайти на созвон", "Write the launch notes"]);
    let mut inner = state.inner.lock().unwrap();
    inner.companion.view.preferences.automatic_completion = true;
    let base = Instant::now();
    let meet = window(
        "Google Chrome",
        "Meet - abc-defg-hij",
        Some("meet.google.com"),
    );
    run(&mut inner, &meet, None, base, 0, 45);
    assert_eq!(
        status(&inner, ids[0]),
        "open",
        "the lobby moment is not enough"
    );
    run(&mut inner, &meet, None, base, 48, 66);
    assert_eq!(status(&inner, ids[0]), "completed");
    assert_eq!(status(&inner, ids[1]), "open");
    assert_eq!(outcomes(&inner), vec!["completion_applied"]);
    // Staying in the call does not close it again.
    run(&mut inner, &meet, None, base, 69, 150);
    assert_eq!(outcomes(&inner), vec!["completion_applied"]);
}

#[test]
fn two_open_call_goals_or_a_disabled_setting_never_close_on_their_own() {
    let (state, ids) = setup(&["Зайти на созвон с HR", "Join the design meeting"]);
    let mut inner = state.inner.lock().unwrap();
    inner.companion.view.preferences.automatic_completion = true;
    let base = Instant::now();
    let zoom = window("zoom.us", "Zoom Meeting", None);
    run(&mut inner, &zoom, None, base, 0, 120);
    assert!(ids.iter().all(|id| status(&inner, *id) == "open"));

    let (state, ids) = setup(&["Visit linear.app"]);
    let mut inner = state.inner.lock().unwrap();
    inner.companion.view.preferences.automatic_completion = false;
    let site = window("Google Chrome", "Linear", Some("linear.app"));
    run(&mut inner, &site, None, base, 0, 12);
    assert_eq!(status(&inner, ids[0]), "open");
    assert_eq!(outcomes(&inner), vec!["automatic_completion_disabled"]);
}

#[test]
fn leaving_a_goal_after_real_work_offers_to_close_it_once() {
    let (state, ids) = setup(&["Write the launch notes"]);
    let mut inner = state.inner.lock().unwrap();
    let base = Instant::now();
    let editor = window("Editor", "Launch notes", None);
    let chat = window("Telegram", "Telegram", None);
    // A short visit is not an episode worth closing.
    run(&mut inner, &editor, Some(ids[0]), base, 0, 30);
    run(&mut inner, &chat, None, base, 33, 200);
    assert!(outcomes(&inner).is_empty());
    // Ninety seconds of work, a two-minute chat, and the work continues: no offer yet.
    run(&mut inner, &editor, Some(ids[0]), base, 203, 293);
    run(&mut inner, &chat, None, base, 296, 416);
    run(&mut inner, &editor, Some(ids[0]), base, 419, 440);
    assert!(outcomes(&inner).is_empty());
    // Away for longer than the episode gap: offered once, and not again soon.
    run(&mut inner, &chat, None, base, 443, 620);
    assert_eq!(outcomes(&inner), vec!["completion_proposed"]);
    run(&mut inner, &editor, Some(ids[0]), base, 623, 713);
    run(&mut inner, &chat, None, base, 716, 900);
    assert_eq!(outcomes(&inner), vec!["completion_proposed"]);
    assert_eq!(
        status(&inner, ids[0]),
        "open",
        "an offer never closes the goal"
    );
}

#[test]
fn an_email_goal_closes_when_the_mail_client_shows_the_message_was_sent() {
    let (state, ids) = setup(&["Написать письмо HR"]);
    let mut inner = state.inner.lock().unwrap();
    inner.companion.view.preferences.automatic_completion = true;
    let base = Instant::now();
    let gmail = window("Google Chrome", "Inbox - Gmail", Some("mail.google.com"));
    let chat = window("Telegram", "Telegram", None);
    crate::companion::screen_context::cache_for_test(&mut inner, &gmail, "Compose\nTo: HR");
    run(&mut inner, &gmail, None, base, 0, 9);
    assert_eq!(status(&inner, ids[0]), "open", "composing is not sending");
    // The same phrase in a chat app is not a sent email.
    crate::companion::screen_context::cache_for_test(&mut inner, &chat, "Message sent");
    run(&mut inner, &chat, None, base, 12, 18);
    assert_eq!(status(&inner, ids[0]), "open");
    crate::companion::screen_context::cache_for_test(
        &mut inner,
        &gmail,
        "Inbox\nMessage sent. Undo View message",
    );
    run(&mut inner, &gmail, None, base, 21, 24);
    assert_eq!(status(&inner, ids[0]), "completed");
    assert_eq!(outcomes(&inner), vec!["completion_applied"]);
}
