//! Local completion signals. Some goals are done the moment something observable happens:
//! joining a call, opening a site, sending an email, submitting a form. A result closes a
//! goal automatically (with a notice and Undo, and no model call) only when it connects to
//! the goal's own target from its understanding: the requested site, the named meeting,
//! the right recipient, the right form. A visible result with no known target is only
//! offered; one that contradicts the target (another meeting, another recipient) is
//! ignored. Every other goal is offered for closing once a work episode on it ends. All
//! decisions use the foreground app, title, site and already read interface text.
use crate::{
    commands::Inner,
    decisions::{Decision, DecisionKind},
    episode_evidence::{FORM_DONE, FORM_ERROR, IN_CALL, MESSAGE_SENT, PRE_JOIN},
    goal_understanding::FastKind,
    models::ActivitySnapshot,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// Time in a meeting window before a "join the call" goal counts as done.
const CALL_DWELL: Duration = Duration::from_secs(60);
/// Time on the named site before a "visit the site" goal counts as done.
const SITE_DWELL: Duration = Duration::from_secs(5);
/// Away from a goal this long ends its work episode.
const EPISODE_END: Duration = Duration::from_secs(150);
/// Attributed time an episode needs before closing the goal is offered.
const EPISODE_MIN: Duration = Duration::from_secs(60);
/// Samples further apart than this are a pause, not continuous time.
const MAX_TICK: Duration = Duration::from_secs(15);

/// What finishing a goal looks like, read from its own words.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GoalKind {
    JoinCall,
    VisitSite(String),
    SendEmail,
    Other,
}
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '-'))
        .map(|w| w.trim_matches(|c| c == '.' || c == '-').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}
fn has(words: &[String], stems: &[&str]) -> bool {
    words
        .iter()
        .any(|w| stems.iter().any(|stem| w.starts_with(stem)))
}
/// Classifies a goal title (and description) in English or Russian. Only explicit
/// phrasing qualifies: "prepare for the call" is not "join the call".
pub(crate) fn classify(title: &str, description: &str) -> GoalKind {
    let all = words(&format!("{title} {description}"));
    let join = has(
        &all,
        &[
            "join",
            "attend",
            "зайти",
            "зайди",
            "войти",
            "подключ",
            "присоедин",
            "попасть",
            "быть",
        ],
    );
    let call = has(
        &all,
        &[
            "call",
            "meeting",
            "meet",
            "zoom",
            "teams",
            "webinar",
            "huddle",
            "созвон",
            "звон",
            "встреч",
            "митинг",
            "конференц",
            "вебинар",
        ],
    );
    let prepare = has(
        &all,
        &["prepar", "подготов", "plan", "заплан", "schedul", "назнач"],
    );
    if join && call && !prepare {
        return GoalKind::JoinCall;
    }
    let visit = has(
        &all,
        &[
            "visit",
            "open",
            "зайти",
            "зайди",
            "открыть",
            "посетить",
            "перейти",
        ],
    );
    let site = all.iter().position(|w| {
        ["site", "website", "page", "сайт", "страниц"]
            .iter()
            .any(|s| w.starts_with(s))
    });
    let domain = all
        .iter()
        .find(|w| {
            w.contains('.')
                && w.rsplit('.').next().is_some_and(|tld| {
                    (2..=10).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_alphabetic())
                })
        })
        .cloned();
    if visit {
        if let Some(domain) = domain {
            return GoalKind::VisitSite(domain.trim_start_matches("www.").to_string());
        }
        if let Some(name) = site.and_then(|at| all.get(at + 1)).filter(|w| {
            w.chars().count() >= 3 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }) {
            return GoalKind::VisitSite(name.clone());
        }
    }
    let send = has(
        &all,
        &["send", "write", "reply", "отправ", "напис", "ответ"],
    );
    let mail = has(
        &all,
        &[
            "email",
            "e-mail",
            "mail",
            "letter",
            "письм",
            "почт",
            "имейл",
        ],
    );
    if send && mail {
        return GoalKind::SendEmail;
    }
    GoalKind::Other
}
/// Meeting surfaces across platforms: web meeting sites, and meeting windows of desktop
/// apps recognised by their title. A chat app's main window is not a call.
pub(crate) fn in_meeting(a: &ActivitySnapshot) -> bool {
    let title = a.window_title.to_lowercase();
    if let Some(domain) = a.browser.as_ref().and_then(|b| b.domain.as_deref()) {
        let domain = domain.to_lowercase();
        let host = |suffix: &str| domain == suffix || domain.ends_with(&format!(".{suffix}"));
        if host("meet.google.com")
            || host("meet.jit.si")
            || host("whereby.com")
            || host("webex.com")
            || host("gather.town")
            || host("around.co")
            || (host("zoom.us") && (title.contains("meeting") || title.contains("webinar")))
            || ((host("teams.microsoft.com") || host("teams.live.com"))
                && (title.contains("meeting") || title.contains("call")))
        {
            return true;
        }
    }
    let app = a.process_name.to_lowercase();
    let says = |phrases: &[&str]| phrases.iter().any(|p| title.contains(p));
    let call_words = [
        "meeting",
        "call",
        "huddle",
        "webinar",
        "звонок",
        "вызов",
        "встреча",
        "конференция",
    ];
    if app.contains("zoom") {
        return says(&[
            "zoom meeting",
            "zoom webinar",
            "конференция zoom",
            "meeting",
        ]);
    }
    if app.contains("facetime") {
        return true;
    }
    if app.contains("webex") {
        return says(&call_words);
    }
    [
        "teams", "slack", "discord", "skype", "telegram", "whatsapp", "viber", "signal",
    ]
    .iter()
    .any(|name| app.contains(name))
        && says(&call_words)
}
/// Mail clients on both platforms: web mail by site, desktop mail by app name.
fn mail_window(a: &ActivitySnapshot) -> bool {
    if let Some(domain) = a.browser.as_ref().and_then(|b| b.domain.as_deref()) {
        let domain = domain.to_lowercase();
        return [
            "mail.google.com",
            "outlook.live.com",
            "outlook.office.com",
            "outlook.office365.com",
            "mail.yahoo.com",
            "mail.yandex.ru",
            "mail.yandex.com",
            "e.mail.ru",
            "mail.proton.me",
            "app.fastmail.com",
            "icloud.com",
        ]
        .iter()
        .any(|host| domain == *host || domain.ends_with(&format!(".{host}")));
    }
    let app = a.process_name.to_lowercase();
    [
        "mail",
        "outlook",
        "spark",
        "thunderbird",
        "airmail",
        "superhuman",
        "mimestream",
    ]
    .iter()
    .any(|name| app.contains(name))
}
/// Whether a mail client shows that a message has left.
fn email_sent(text: &str) -> bool {
    crate::episode_evidence::says(text, MESSAGE_SENT)
}
fn site_matches(a: &ActivitySnapshot, target: &str) -> bool {
    let Some(domain) = a
        .browser
        .as_ref()
        .and_then(|b| b.domain.as_deref())
        .map(str::to_lowercase)
    else {
        return false;
    };
    let target = target.to_lowercase();
    if target.contains('.') {
        domain == target || domain.ends_with(&format!(".{target}"))
    } else {
        domain
            .split('.')
            .any(|label| label == target || label.trim_end_matches("-app") == target)
    }
}
fn says(text: Option<&str>, phrases: &[&str]) -> bool {
    text.is_some_and(|t| phrases.iter().any(|p| t.contains(p)))
}
/// Kind and targets: the stored understanding first, the title classifier as fallback.
fn shape(
    inner: &Inner,
    goal: &crate::core::CoreGoal,
) -> Result<(Option<FastKind>, Vec<String>), String> {
    if let Some(u) = inner.storage.understanding_for(goal)? {
        let mut targets = u.targets.clone();
        targets.extend(u.entities.iter().cloned());
        return Ok((u.kind, targets));
    }
    Ok(match classify(&goal.title, &goal.description) {
        GoalKind::JoinCall => (Some(FastKind::JoinCall), vec![]),
        GoalKind::VisitSite(site) => (Some(FastKind::VisitSite), vec![site]),
        GoalKind::SendEmail => (Some(FastKind::SendEmail), vec![]),
        GoalKind::Other => (None, vec![]),
    })
}
/// Whether the window or its text names one of the goal's targets.
fn names_target(sample: &ActivitySnapshot, text: Option<&str>, targets: &[String]) -> bool {
    let title = sample.window_title.to_lowercase();
    let site = sample
        .browser
        .as_ref()
        .and_then(|b| b.domain.as_deref())
        .unwrap_or_default()
        .to_lowercase();
    targets
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| t.chars().count() >= 2)
        .any(|t| title.contains(&t) || site.contains(&t) || text.is_some_and(|x| x.contains(&t)))
}
/// The observed result for one goal: its signal, how long it must hold, and whether it
/// connects to the goal's own target. `None` when nothing relevant is visible or the
/// evidence contradicts the target (another meeting, another recipient).
fn evidence(
    inner: &Inner,
    goal: &crate::core::CoreGoal,
    sample: &ActivitySnapshot,
    text: Option<&str>,
    fresh: &Fresh,
) -> Result<Option<(&'static str, Duration, bool)>, String> {
    let (kind, targets) = shape(inner, goal)?;
    let linked = names_target(sample, text, &targets);
    // Known targets must match; without any, the result can only be offered.
    let result = |signal, needed| {
        if targets.is_empty() {
            Some((signal, needed, false))
        } else if linked {
            Some((signal, needed, true))
        } else {
            None
        }
    };
    Ok(match kind {
        Some(FastKind::JoinCall)
            if in_meeting(sample) && !(says(text, PRE_JOIN) && !says(text, IN_CALL)) =>
        {
            result("meeting", CALL_DWELL)
        }
        Some(FastKind::VisitSite) if targets.iter().any(|t| site_matches(sample, t)) => {
            Some(("site", SITE_DWELL, true))
        }
        Some(FastKind::SendEmail) if fresh.sent && mail_window(sample) => {
            result("email_sent", Duration::ZERO)
        }
        Some(FastKind::SubmitForm) if fresh.submitted && sample.browser.is_some() => {
            result("form_submitted", Duration::ZERO)
        }
        _ => None,
    })
}
/// Result phrases that appeared in this window since its previous sample. A confirmation
/// already on screen at the first sample (an old or quoted one) does not count.
#[derive(Default)]
struct Fresh {
    sent: bool,
    submitted: bool,
}
fn fresh_signals(inner: &mut Inner, sample: &ActivitySnapshot, text: Option<&str>) -> Fresh {
    let key = format!(
        "{}|{:?}|{:?}",
        sample.process_name.to_lowercase(),
        sample.window_id,
        sample.browser.as_ref().and_then(|b| b.domain.clone())
    );
    let sent = text.is_some_and(email_sent);
    let submitted = says(text, FORM_DONE) && !says(text, FORM_ERROR);
    let seen = &mut inner.goal_matching.signals.seen_text;
    let previous = text.and_then(|_| seen.insert(key.clone(), (sent, submitted)));
    if seen.len() > 64 {
        seen.retain(|k, _| *k == key);
    }
    match previous {
        Some((was_sent, was_submitted)) => Fresh {
            sent: sent && !was_sent,
            submitted: submitted && !was_submitted,
        },
        None => Fresh::default(),
    }
}
/// The goal's current plan revision, or `None` when it is no longer open.
fn revision(inner: &Inner, goal: i64) -> Result<Option<u64>, String> {
    Ok(inner
        .storage
        .core_work_snapshot()?
        .goals
        .into_iter()
        .find(|g| g.id == goal && g.status == "open")
        .map(|g| g.plan.revision))
}
/// A visible result that cannot be tied to the goal's target is offered, never closed.
fn offer_unlinked(inner: &mut Inner, goal: i64, now: Instant) -> Result<(), String> {
    inner.goal_matching.signals.dwell = None;
    let Some(revision) = revision(inner, goal)? else {
        return Ok(());
    };
    crate::decisions::submit_at(
        inner,
        Decision {
            goal_id: Some(goal),
            step_id: None,
            kind: DecisionKind::ProposeComplete,
            evidence: vec![],
            reason: String::new(),
            confidence: 0.7,
            dedupe_key: format!("fast_offer:{goal}:{revision}"),
            source: "fast_signal".into(),
            text: None,
            place: None,
            plan_revision: Some(revision),
        },
        now,
    )?;
    Ok(())
}
#[derive(Clone)]
struct Dwell {
    goal: i64,
    signal: &'static str,
    since: Duration,
}
#[derive(Clone)]
struct Episode {
    attributed: Duration,
    started: Instant,
    last: Instant,
}
/// Per-session signal state; nothing here is persisted.
#[derive(Default)]
pub struct Tracker {
    last_tick: Option<Instant>,
    dwell: Option<Dwell>,
    episodes: HashMap<i64, Episode>,
    /// Per window: whether the sent and submitted phrases were already visible.
    seen_text: HashMap<String, (bool, bool)>,
}
/// One collection tick. `attributed` is the goal the current sample counts toward, if any.
pub(crate) fn observe(
    inner: &mut Inner,
    sample: &ActivitySnapshot,
    allowed: bool,
    attributed: Option<i64>,
    now: Instant,
) -> Result<(), String> {
    let tracker = &mut inner.goal_matching.signals;
    let step = tracker
        .last_tick
        .map(|at| now.saturating_duration_since(at))
        .filter(|d| *d <= MAX_TICK)
        .unwrap_or_default();
    tracker.last_tick = Some(now);
    let goals: Vec<_> = inner
        .storage
        .core_work_snapshot()?
        .goals
        .into_iter()
        .filter(|g| g.status == "open")
        .collect();
    if allowed {
        let text =
            crate::companion::screen_context::cached_text(inner, sample).map(str::to_lowercase);
        let fresh = fresh_signals(inner, sample, text.as_deref());
        let mut found: Vec<(i64, &'static str, Duration, bool)> = Vec::new();
        for goal in &goals {
            if let Some((signal, needed, connected)) =
                evidence(inner, goal, sample, text.as_deref(), &fresh)?
            {
                found.push((goal.id, signal, needed, connected));
            }
        }
        // A connected result decides alone; otherwise exactly one candidate may be offered.
        let connected: Vec<_> = found.iter().filter(|f| f.3).cloned().collect();
        let candidate = match (connected.as_slice(), found.as_slice()) {
            ([one], _) => Some(*one),
            ([], [one]) => Some(*one),
            _ => None,
        };
        let tracker = &mut inner.goal_matching.signals;
        let dwell = match (candidate, tracker.dwell.take()) {
            (Some((goal, signal, _, _)), Some(d)) if d.goal == goal && d.signal == signal => {
                Dwell {
                    since: d.since + step,
                    ..d
                }
            }
            (Some((goal, signal, _, _)), _) => Dwell {
                goal,
                signal,
                since: Duration::ZERO,
            },
            (None, _) => {
                tracker.dwell = None;
                return offer_ended_episodes(inner, sample, attributed, step, now);
            }
        };
        let (_, _, needed, linked) = candidate.unwrap_or((0, "", CALL_DWELL, false));
        if dwell.since >= needed {
            if linked {
                close(inner, dwell.goal, dwell.signal, sample, now)?;
            } else {
                offer_unlinked(inner, dwell.goal, now)?;
            }
        } else {
            inner.goal_matching.signals.dwell = Some(dwell);
        }
    } else {
        inner.goal_matching.signals.dwell = None;
    }
    offer_ended_episodes(inner, sample, attributed, step, now)
}
/// Closes a goal whose result connects to its target, through the shared decision path,
/// which keeps a durable Undo and respects the automatic-completion setting.
fn close(
    inner: &mut Inner,
    goal: i64,
    signal: &str,
    sample: &ActivitySnapshot,
    now: Instant,
) -> Result<(), String> {
    let Some(revision) = revision(inner, goal)? else {
        return Ok(());
    };
    let place = sample
        .browser
        .as_ref()
        .and_then(|b| b.domain.clone())
        .unwrap_or_else(|| sample.process_name.to_lowercase());
    let applied = crate::decisions::submit_at(
        inner,
        Decision {
            goal_id: Some(goal),
            step_id: None,
            kind: DecisionKind::AutoComplete,
            evidence: vec![],
            reason: String::new(),
            confidence: crate::work_journal::MIN_VERIFIED_CONFIDENCE,
            dedupe_key: format!("fast:{goal}:{revision}:{signal}:{place}"),
            source: "fast_signal".into(),
            text: None,
            place: Some(sample.process_name.clone()),
            plan_revision: Some(revision),
        },
        now,
    )?;
    if applied == crate::decisions::Applied::Changed {
        inner.goal_matching.signals.episodes.remove(&goal);
    }
    Ok(())
}
/// Tracks attributed time per goal and, once the user has been away from a goal long
/// enough, offers to close it. The user decides; "Not yet" simply lets it go.
fn offer_ended_episodes(
    inner: &mut Inner,
    sample: &ActivitySnapshot,
    attributed: Option<i64>,
    step: Duration,
    now: Instant,
) -> Result<(), String> {
    // A window sharing a goal's own words is not proof of leaving it, even before the
    // matcher has confirmed it: the episode stays open without gaining time.
    let open: Vec<(i64, String, bool)> = if inner.goal_matching.signals.episodes.is_empty() {
        Vec::new()
    } else {
        inner
            .storage
            .core_work_snapshot()?
            .goals
            .into_iter()
            .filter(|g| inner.goal_matching.signals.episodes.contains_key(&g.id))
            .map(|g| (g.id, format!("{} {}", g.title, g.description), false))
            .collect()
    };
    let related = crate::relevance::hints(&open, sample);
    let tracker = &mut inner.goal_matching.signals;
    for goal in related {
        if let Some(episode) = tracker.episodes.get_mut(&goal) {
            episode.last = now;
        }
    }
    if let Some(goal) = attributed {
        let episode = tracker.episodes.entry(goal).or_insert(Episode {
            attributed: Duration::ZERO,
            started: now,
            last: now,
        });
        episode.attributed += step;
        episode.last = now;
    }
    let ended: Vec<(i64, Episode)> = tracker
        .episodes
        .iter()
        .filter(|(_, e)| now.saturating_duration_since(e.last) >= EPISODE_END)
        .map(|(id, e)| (*id, e.clone()))
        .collect();
    // With the model available, the end-of-episode review decides with a factual reason;
    // the time-based offer is the fallback when it is not.
    let reviewed = crate::episode_review::reviews_episodes(inner);
    for (goal, episode) in ended {
        inner.goal_matching.signals.episodes.remove(&goal);
        if episode.attributed < EPISODE_MIN || reviewed {
            continue;
        }
        let wall = chrono::Utc::now().timestamp();
        let started = wall - now.saturating_duration_since(episode.started).as_secs() as i64;
        crate::decisions::submit_at(
            inner,
            Decision {
                goal_id: Some(goal),
                step_id: None,
                kind: DecisionKind::ProposeComplete,
                evidence: vec![],
                reason: String::new(),
                confidence: 0.7,
                dedupe_key: format!("episode_time:{goal}:{started}"),
                source: "episode_time".into(),
                text: None,
                place: None,
                plan_revision: None,
            },
            now,
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests;
