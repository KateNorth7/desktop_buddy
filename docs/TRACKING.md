# Tracking implementation (0.8.0)

> Historical milestone / design record. This file describes its recorded source and may contain superseded UI, models, permissions or commands. For current behavior and diagrams, use the [documentation index](README.md) and [architecture](AI_ARCHITECTURE.md). Historical test results are not current live verification.


## Audit of the two briefs

Neither brief was fully implemented in the original 0.7.1 checkout. Prompt 1 had no first-run flow, role presets, name/email profile or character selector. Prompt 2 had a foreground process/window-title collector, a 60-second idle cutoff, sampled daily app usage, per-goal app rules and persisted recommendations. It lacked address-bar adapters, deterministic local activity states, working hours and a completed-goal analytics API. The user selected **Prompt 2** for this implementation. Onboarding and avatar UI are left for the other brief.

## Browser metadata and privacy

`BrowserActivityProvider` augments the existing Windows collector; it is not a separate tracking engine. Chrome and Edge receive a foreground-window title and best-effort domain. Firefox uses the same adapter when its `urlbar-input` accessibility element is exposed. Every recognized browser has a title-only fallback with an explicit `source` (`window_title` or `address_bar`). `window_id` identifies the observed foreground window within a running Windows session.

The adapter uses `GetForegroundWindow`, `GetWindowTextW`, `GetWindowThreadProcessId`, `QueryFullProcessImageNameW`, `GetLastInputInfo`, and Windows UI Automation (`CUIAutomation8`, `ElementFromHandle`, control-view traversal, `IUIAutomationValuePattern`). A bounded COM worker reads only a known address-bar control. It skips document subtrees, password controls, focused address bars (which may contain unfinished typed text) and offscreen controls. It never focuses or changes a window or requests keystroke contents. Control traversal, queue depth and caller waiting are bounded; late replies are discarded. Foreground/window-title changes during sampling invalidate the sample.

Only a normalized public HTTP(S) hostname survives the address-bar adapter. URL paths, query strings, fragments, embedded credentials, internal browser pages, local addresses and address-bar search text are not retained. Recognizable address tokens in browser titles are also reduced to hostnames or omitted. Titles are limited to 160 characters and may still contain sensitive user-defined text. The address-bar metadata adapter does not read page bodies, forms, clipboard or screenshots. Existing exclusions prevent address-bar probing for excluded apps. Turning off **Browser domain metadata** prevents address-bar reads on subsequent samples; existing retained metadata follows the normal retention/deletion controls. Automatic goal matching may send the collected domain and bounded page title to Nebius as described below.

Address-bar accessibility depends on browser/version/language and Windows permissions. Chrome/Edge support recognized English, Russian and Hebrew address-bar names; Firefox's ID is preferred. Unknown/localized controls safely fall back to titles. Fullscreen, protected windows and edited address bars may provide no domain. Distinct pages sharing a domain and title cannot be distinguished; there is no stable browser tab ID, browser extension or DOM inspection. Swapping tabs or URLs between samples can be missed.

References: [Microsoft UI Automation element lookup](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-obtainingelements), [ElementFromHandle](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation-elementfromhandle).

## Cloud goal matching and opted-in visible text

With automatic matching and AI goal-context sharing enabled, Nebius receives the open-goal candidates, foreground app, window title (first 160 characters), and available browser domain and page title (first 160 characters). Missing browser fields are `null`; full address-bar URLs, paths, queries and fragments are not added.

Only while **Suggest completed work from what is on screen** (`screen_task_detection`) is enabled, matching may also include up to **1,200 characters** of redacted visible window text, taken from the start and the end of the sample (the opening of a document and the newest lines of a conversation), with the omitted middle marked. The native sample itself skips application chrome (navigation, banner, footer and search landmarks, toolbars and menus) and prefers a web app's main landmark, so a sidebar listing other conversations is not mistaken for the work. Windows that share one title (chat apps, notes, editors) are told apart locally by a hash of their document identity (an editor's file, a web app's document URL without query or fragment); the raw identity is never stored or disclosed. Without this preference, without available text, or for a sensitive/excluded app or window, `visible_text` is `null`. Matching uses the same Accessibility sampling path as completion detection, with selected-text capture disabled. Password and editable controls are skipped by the Windows/macOS adapters, password-manager apps and sensitive windows are excluded, and secret-bearing lines (including `api_key`) are omitted before truncation. Screen text is untrusted evidence; instructions in it must be ignored.

The two consumers share one serialized read and a transient, redacted sample of at most 3,000 characters. A sample from the same foreground window ID, app, title and browser context is reused for up to 60 seconds. Failed/unavailable reads are shared too. Changes to consent, opt-out or the foreground context prevent reuse; an in-flight read is rechecked before its result can be sent. The cache stays in memory; the sample itself is not written to disk, SQLite or diagnostics. Matching does not add a second Accessibility read in the same tick. Existing matching confidence thresholds, debounce, request cooldowns and completion-check cadence are unchanged. No screenshots, clipboard access or keystroke recording are introduced.

## State and avatar contract

The existing dashboard's `buddy` object adds `activity_state`, `activity_event`, and monotonically increasing `activity_revision` (within the app process). The frontend may use the revision to consume each transition once. Persisted events remain goal-specific; avatar components are not rewritten.

| Signal | State/event |
| --- | --- |
| Tracking enabled with an allowed context and recent input | `focused` / `working` on the first observation |
| Same app/window/title/domain for at least 180 seconds, no input for at least 120 seconds, no confirmed media | `drifting` / `drifting` |
| No keyboard/mouse input for 300 seconds by default | `paused` / `paused` |
| Input resumes after idle/drifting | `focused` / `resumed` |
| Explicit pause, excluded context or collector failure | `paused` |
| Goal completion | `paused` / `goal_completed` |

An unchanged context with no input can also mean reading. The detector describes observed signals, not emotions or certain distraction. Context switches, gaps over 15 seconds and goal switches reset its timer. Local drifting cards require the existing **Local distraction reminders** opt-in and share the current cooldown, budget, DND, snooze, meeting and fullscreen controls. They do not call a provider.

## Media and working hours

`GlobalSystemMediaTransportControlsSessionManager` provides a separate confirmed-media signal. Only a `Playing` session reporting an exact matching executable identity counts. A playing session from the foreground process suppresses automatic cards and drifting, including when that process has a background tab playing media. This deliberately errs on the side of quiet. It cannot identify the playing tab, distinguish audio from video or detect players that do not expose a matching system session. Titles alone never establish playback. The normal idle threshold still applies during media playback.

Reference: [Microsoft global media sessions](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssession).

`TrackingSettings` is stored as `preferences.tracking_settings`, independently of `BuddyPreferences`, with serde defaults for missing fields. `get_tracking_settings` / `set_tracking_settings` provide the shared API. Settings exposes working hours (09:00–18:00 by default), idle threshold and domain metadata in the existing Settings screen. Idle and drifting thresholds are validated and configurable through the API. Overnight ranges are supported; start is inclusive, end exclusive, in the computer's local time zone. Equal start/end is rejected.

All automatic local reminders, AI check-ins, resource suggestions and final display decisions respect working hours, idle and media signals. Tracking continues outside hours. Explicit manual checks/searches remain available. Saving tracking settings invalidates pending analyses and suggestions and resets interval boundaries.

## Time and history API

`UsageTracker` now returns disjoint observed intervals, attributed to one goal and one app/window/title/domain. It rejects crossing context/state/goal changes, gaps over 15 seconds, wall-clock jumps and restart boundaries. Transition intervals are intentionally omitted because the switch time is unknown. It never infers sleep duration or time while the app was closed. SQLite rejects overlapping intervals and commits detailed history and daily active totals together. Observed idle intervals are persisted separately and excluded from active totals.

`get_goal_history({ goalId: null })` / `api.goalHistory()` returns the latest 100 completed goals; passing a goal ID returns its retained history, including active/deferred goals. Each record includes:

- Goal ID/text, creation/completion times, elapsed wall time when completion is known, and observed coverage bounds.
- Active, focused, observed idle and drifting milliseconds.
- Previously measured daily app totals remain part of active/app time. `unattributed_active_milliseconds` identifies retained aggregate time without detailed site/state intervals (legacy totals or detail removed by retention); it is never assigned to invented sites or focused/idle states.
- Work-app and distraction-app milliseconds based on the rule at observation time, avoiding retrospective reclassification.
- Local drifting and pause event counts, and intervention decisions (distinct metrics, not summed together).
- App, domain and title/domain groupings, plus all retained recommendations for this goal with feedback.

Focused time means observable engagement, not verified productive work. Work-app time is an explicit user-category proxy. Manual pauses, deferred periods, sleep, shutdown time and uncertain transitions have no inferred active/idle duration; elapsed goal wall time may include them. Start time is goal creation, not the start of continuous tracking. Tab groupings use title/domain metadata, not unique tab IDs. Retention can remove historical coverage and recommendations. No dedicated Goal History UI is added: the typed API is ready for that screen.

## Additive migrations

- Nullable `goals.completed_at` (legacy completed goals keep an unknown end time).
- Nullable `activity.browser_context` and `activity.window_id`, plus `media_playing` defaulting to false.
- `usage_intervals` with context, state, category-at-observation and media flag; indexed by goal.
- `tracking_events` with goal/time/event; indexed by goal.

Migrations are idempotent. Old activity rows and daily summaries remain readable. Cumulative activity seconds are never backfilled into new interval history. Retention and explicit deletion cover both new tables; surviving recent intervals/events prevent premature deletion of their goal.

## Changed files

New modules: `src-tauri/src/browser.rs`, `tracking.rs`, `tracking_tests.rs`, `history.rs`, `collector/browser_windows.rs`; frontend `src/components/TrackingSettings.tsx` and its tests; this document.

Shared/core changes: `src-tauri/src/models.rs`, `commands.rs`, `storage.rs`, `insights.rs`, `goals.rs`, `buddy.rs`, `attention.rs`, `privacy.rs`, `lib.rs`, `collector/mod.rs`, `collector/windows.rs`, `src/types.ts`, `src/api/tauri.ts`. These add optional metadata, state/settings contracts, interval persistence, goal-completion timestamps, notification guards and lifecycle resets.

Small integration changes: `Settings.tsx`, `ActivityTimeline.tsx`, `ActivityInsights.tsx`. Updated regression fixtures: `buddy_tests.rs`, `insights_tests.rs`, `privacy_tests.rs`, `recommendation_provider_tests.rs`, `threading_tests.rs`, `src/api/tauri.test.ts`. Cargo Windows feature flags and all five release-version manifests/lockfiles are synchronized to 0.8.0. README and CHANGELOG describe the behavior.

## Manual Windows QA

1. Run `./scripts/dev.ps1`. Start a goal; enable domain metadata in Settings. Open Chrome/Edge, load two different public sites and blur the address bar. Verify the activity timeline shows the foreground title and correct domain where accessibility is available.
2. Switch app → tab A → tab B → app, including two windows and pages sharing a title. Confirm totals never increase twice for one interval. Edited/internal address bars should show title-only metadata.
3. Remain in one context without input: after 3 minutes observe `drifting`; after 5 minutes observe `paused`. Move the mouse/type and observe `resumed`. With local reminders disabled, no local intervention should appear.
4. Play media in a supported app/browser that exposes an exact executable media session. Confirm cards stay suppressed and state does not drift before the idle threshold. Also test a player with no accessible media session; it is an explicitly unsupported case.
5. Set working hours outside the current local time; exercise local reminders, automatic AI and proactive suggestions. Confirm no automatic card appears. Restore hours and verify normal budget/consent controls.
6. Pause/resume manually, sleep/wake, restart the app, defer/resume a goal, and complete it. Tracking starts paused after restart; unobserved gaps are not counted. Use `get_goal_history` to inspect milliseconds and completion time.
7. Generate a recommendation through the existing consented flow; restart and verify it stays associated with the original goal's history. Check browser metadata never appears as a full URL in SQLite. Test retention and Clear local history against a disposable database.
8. Reopen a copy of a 0.7.1 database. Confirm old goals/activity/recommendations remain readable and new history begins at new observations, without invented past site/idle totals.

Automated tests cover state transitions, settings persistence and boundaries, URL minimization, disjoint usage, migrations, completion/restart, history/recommendation persistence and notification suppression. Native browser behavior across browser versions/languages and real playback remains a manual QA item.

Verified in this checkout: 74 Rust tests passed (one Credential Manager integration test intentionally ignored), 32 frontend tests passed, TypeScript/Vite production build passed, `cargo build --locked` produced the Windows executable, Rustfmt/Prettier and `git diff --check` passed. No lint script is defined in `package.json`. Rust was installed locally under ignored `.tools`; Visual Studio C++ Build Tools and Windows SDK were installed with the user's explicit approval. Run `./scripts/dev.ps1` to activate the native build environment. A release installer and live browser/media manual QA were not produced by these checks.

## Unlisted work in Today

With the existing on-screen task-detection opt-in and AI goal-context sharing enabled,
sumi can suggest work that is absent from the goal list. It uses the same protected
Accessibility sample on Windows and macOS: no screenshots, clipboard or keystrokes,
no password/edit-control capture, the same sensitive-window/app exclusions and
secret-line redaction. Only a 1,200-character excerpt, minimized app/window/browser
metadata and existing goal titles/unfinished steps are sent to Nebius. The text is
untrusted evidence, never authorization. Raw excerpts remain transient and are
not stored in the activity journal, diagnostics or the completed goal.

The collector journals actual active intervals within each continuous foreground
session, including activity without a selected goal. Returning to the same app
later starts a separate session; a new screen excerpt cannot relabel earlier work. It uses existing goal-owned intervals where available and the same disjoint
UsageTracker boundaries otherwise. Idle/media/meeting contexts, excluded/sensitive
windows, manual timer sessions, gaps over 15 seconds and midnight transition
intervals are omitted. Neither model estimates nor app downtime contribute time.
After at least two observed active minutes and 60 seconds in a stable foreground
context, a classifier may identify concrete unlisted work. It compares every open/
deferred goal plus goals completed in Today, including unfinished steps. Unclear
context, a plausible existing goal, entertainment or confidence below 0.85 stays
silent. Requests are limited to one per five minutes, share the existing provider
lock, respect quiet/working-hour rules, and use one reasoning-enabled HTTP disclosure with a 30-second bound and
consent and context checks before sending and before offering.

Today shows a persistent **Work noticed** card with an editable title and the
observed duration so far. **Add to completed** confirms that observed work as a
completed Today goal, without selecting it as focus or starting a timer. The
backend owns the offer and duration; the frontend cannot supply intervals or time.
Existing interval/daily/step totals are transferred atomically, so the same time
is not counted twice under the previously focused goal. No new goal is created
before confirmation. **Not now** snoozes for ten minutes; **Dismiss** suppresses
that context for the local day. Adding also suppresses repeat offers for that
context that day. Only work observed up to acceptance is attached; later work is
not silently added to a goal the user has already confirmed as completed.

Local metadata-only `unplanned_work_intervals` and day/context review decisions
follow configured history retention and **Clear local history**. Source interval
deletion cascades to unconfirmed references. Completed goals and their time use
the existing goal deletion/history paths. Pending offers are in memory, expire
after 30 minutes/day change, and are revoked by consent or goal-context changes.
The card is recovered when Today opens while the process is running; pending
screen text is never persisted across restart.

Synthetic live check (no installed data or desktop sampling):
`cargo run --locked --manifest-path src-tauri/Cargo.toml -- --check-unplanned-work`.
Native Accessibility coverage still depends on app controls and OS permission;
unavailable text leaves the feature silent.
