use super::*;
use crate::{
    history::UsageInterval, models::ActivitySnapshot, relevance::ActivityMatch,
    tracking::ActivityState,
};

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}
fn window(app: &str, title: &str, domain: Option<&str>) -> ActivitySnapshot {
    let mut a = ActivitySnapshot {
        process_name: app.into(),
        window_title: title.into(),
        window_id: Some(9),
        ..Default::default()
    };
    if let Some(domain) = domain {
        a.browser = crate::browser::context(app, title, Some(domain.into()));
    }
    a
}
fn record(s: &Storage, a: &ActivitySnapshot, from: &str, seconds: i64, goal: Option<i64>) {
    let mut t = at(from);
    let end = t + chrono::Duration::seconds(seconds);
    while t < end {
        let mut sample = a.clone();
        sample.timestamp = t.to_rfc3339();
        let interval = UsageInterval {
            start: t,
            end: t + chrono::Duration::seconds(3),
            activity: sample,
            state: ActivityState::Focused,
        };
        s.record_review_interval(
            &interval,
            &ActivityMatch {
                goal_id: goal.map(|g| g.to_string()),
                confidence: if goal.is_some() { 0.9 } else { 0. },
                reason: "test".into(),
            },
        )
        .unwrap();
        t += chrono::Duration::seconds(3);
    }
}
fn range(from: &str, to: &str) -> EpisodeRange {
    EpisodeRange {
        since: at(from),
        until: at(to),
    }
}

#[test]
fn result_codes_prefer_errors_and_cover_mail_forms_and_calls() {
    assert_eq!(result_signal("Message sent. Undo"), Some("message_sent"));
    assert_eq!(result_signal("Письмо отправлено"), Some("message_sent"));
    assert_eq!(
        result_signal("Thank you for applying"),
        Some("form_submitted")
    );
    assert_eq!(
        result_signal("Something went wrong. Thank you for applying"),
        Some("form_error")
    );
    assert_eq!(result_signal("Leave call  Mute"), Some("in_call"));
    assert_eq!(result_signal("Ready to join? Join now"), Some("pre_join"));
    assert_eq!(result_signal("Quarterly notes"), None);
}

#[test]
fn an_episode_keeps_its_windows_results_and_goal_across_a_restart() {
    let dir = std::env::temp_dir().join(format!(
        "sumi-episode-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("buddy.db");
    let goal = {
        let mut s = Storage::open(&path).unwrap();
        s.core_add_goals(
            vec![crate::core::GoalDraft {
                title: "Reply to the recruiter".into(),
                area: "Work".into(),
                steps: vec![],
            }],
            "episode",
        )
        .unwrap();
        let goal = s.core_work_snapshot().unwrap().goals[0].id;
        let mail = window("Google Chrome", "Inbox - Gmail", Some("mail.google.com"));
        record(&s, &mail, "2026-11-10T09:00:00Z", 30, Some(goal));
        let mut sent = mail.clone();
        sent.timestamp = "2026-11-10T09:00:20Z".into();
        s.retain_review_evidence(&sent, "text", "Message sent. Undo View message")
            .unwrap();
        let chat = window("Telegram", "Telegram", None);
        record(&s, &chat, "2026-11-10T09:00:30Z", 15, None);
        goal
    };
    // A restart reads the same evidence back from the database.
    let s = Storage::open(&path).unwrap();
    let evidence = s
        .episode_evidence(range("2026-11-10T08:59:00Z", "2026-11-10T09:02:00Z"))
        .unwrap();
    assert_eq!(evidence.len(), 2);
    assert_eq!(evidence[0].app, "google chrome");
    assert_eq!(evidence[0].site.as_deref(), Some("mail.google.com"));
    assert_eq!(evidence[0].goal_id, Some(goal));
    assert_eq!(evidence[0].result_signal.as_deref(), Some("message_sent"));
    assert_eq!(evidence[0].provenance, "interface_text");
    assert_eq!(evidence[0].seconds, 30);
    assert_eq!(evidence[1].goal_id, None, "uncertain time stays unassigned");
    assert_eq!(evidence[1].provenance, "window_metadata");
    // The excerpt is sealed, redacted and bounded; the reference alone carries no text.
    let reference = evidence[0].excerpt_ref.clone().unwrap();
    assert!(!reference.contains("Message"));
    let text = s.episode_text(&reference).unwrap().unwrap();
    assert!(text.contains("Message sent"));
    assert!(text.chars().count() <= EXCERPT_CHARS);
    assert_eq!(s.episode_text("review_text:not-a-number:x").unwrap(), None);
    assert_eq!(s.episode_text("elsewhere").unwrap(), None);
    let summaries = s
        .episode_summaries(range("2026-11-10T08:59:00Z", "2026-11-10T09:02:00Z"))
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].goal_ids, vec![goal]);
    assert!(summaries[0]
        .summary
        .contains("mail.google.com 1m (message_sent)"));
    assert!(
        !summaries[0].summary.contains("Message sent"),
        "no screen text in summaries"
    );
    drop(s);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn episodes_split_at_gaps_and_long_spans_and_reads_are_bounded() {
    let s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let editor = window("Editor", "Notes", None);
    record(&s, &editor, "2026-11-10T10:00:00Z", 60, None);
    // Three minutes away: a new episode.
    record(&s, &editor, "2026-11-10T10:04:00Z", 60, None);
    let split = s
        .episode_summaries(range("2026-11-10T09:00:00Z", "2026-11-10T11:00:00Z"))
        .unwrap();
    assert_eq!(split.len(), 2);
    // More distinct windows than a read returns.
    for n in 0..(MAX_ITEMS + 5) {
        let w = window("Editor", &format!("Draft {n}"), None);
        let start = at("2026-11-10T12:00:00Z") + chrono::Duration::seconds(3 * n as i64);
        record(&s, &w, &start.to_rfc3339(), 3, None);
    }
    let many = s
        .episode_evidence(range("2026-11-10T11:59:00Z", "2026-11-10T13:00:00Z"))
        .unwrap();
    assert_eq!(many.len(), MAX_ITEMS);
    let summaries = s
        .episode_summaries(range("2026-11-10T11:59:00Z", "2026-11-10T13:00:00Z"))
        .unwrap();
    assert!(summaries
        .iter()
        .all(|e| e.sources.len() <= SUMMARY_SOURCES && e.summary.chars().count() <= SUMMARY_CHARS));
}

#[test]
fn clear_local_history_removes_episode_evidence() {
    let mut s = Storage::open(std::path::Path::new(":memory:")).unwrap();
    let mail = window("Mail", "Inbox", None);
    record(&s, &mail, "2026-11-10T09:00:00Z", 9, None);
    let mut sample = mail.clone();
    sample.timestamp = "2026-11-10T09:00:03Z".into();
    s.retain_review_evidence(&sample, "text", "Message sent")
        .unwrap();
    let r = range("2026-11-10T08:00:00Z", "2026-11-10T10:00:00Z");
    assert_eq!(s.episode_evidence(r).unwrap().len(), 1);
    s.purge_history(None).unwrap();
    assert!(s.episode_evidence(r).unwrap().is_empty());
    assert!(s.episode_summaries(r).unwrap().is_empty());
}
