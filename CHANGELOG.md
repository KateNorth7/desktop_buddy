# Changelog

## Unreleased — Claude as the inference provider

- Replace Nebius Token Factory with the Claude Messages API (Anthropic). All
  reasoning, dialogue, goal matching, screen understanding and image description
  run on `claude-opus-5-5` by default; `ANTHROPIC_MODEL_ID`,
  `ANTHROPIC_RESPONSE_MODEL_ID` and `ANTHROPIC_VISION_MODEL_ID` override it for
  personal connections. The saved provider key is now `ANTHROPIC_API_KEY`; on
  macOS it occupies the Keychain slot the pinned credential helper already
  accepts, so a previously saved Nebius key must be replaced in Settings.
- Translate every request at the HTTP boundary (`claude.rs`): strict JSON output
  uses Claude structured outputs, system prompts are cached, hidden reasoning runs
  at low effort for bounded operations and high effort for deliberate planning,
  and a safety refusal falls back server-side to another Claude model.
- Route built-in AI through `/anthropic/v1/messages` and check model access with
  one model object instead of a paginated catalog. The Supabase function needs
  an `ANTHROPIC_API_KEY` secret and a redeploy.

## 0.36.11 — grounded research and demo readiness

- Select exact backend-owned source excerpts by ID and validate research citations.
- Keep verified action receipts available when optional AI wording is too slow.
- Distinguish local macOS Dictation and language-asset failures.
- Add an isolated real-provider hero rehearsal and a native demo checklist.
- Preserve the AI connection and Today planning fixes from 0.36.10.

## 0.36.10 — AI reliability and clearer daily planning

- Publish the AI connection, generation timeout, saved-goal search and progress
  improvements validated in the local 0.36.2–0.36.9 builds.
- Choose multiple previous goals for Today before confirming with Done.
- Include the goal-bound coaching fixes from 0.36.1 in the coordinated release.

## 0.36.9 — choose today's goals before confirming

- Keep the initial Today picker open while selecting multiple previous goals.
  Plus buttons toggle selections; Done saves the full selection together.
- Explain that the list contains previous goals and preserve selections if saving fails.

## 0.36.8 — lime progress total in dark mode

- Use the shared lime colour for the My progress time total in dark mode,
  keeping the violet total in light mode.

## 0.36.7 — violet progress total

- Use the shared violet accent for the My progress time total, keeping the
  neutral card surface and normal description text.

## 0.36.6 — neutral progress summary

- Use the shared light-grey card surface and normal primary text for the
  My progress result block, with the existing theme colours in dark mode.

## 0.36.5 — cleaner saved-goal search

- Remove dismiss crosses from saved-goal search results. Clicking a goal row
  still adds it to Today.

## 0.36.4 — simpler saved-goal suggestions

- Remove redundant plus icons from saved-goal suggestions. Clicking a goal row
  continues to add it to Today.

## 0.36.3 — interactive AI latency

- Set Kimi K3 to its supported low reasoning effort for interactive requests,
  instead of inheriting the model's maximum-effort default.
- Report provider timeouts as timeouts, with the saved message retained.
- Preserve provider authentication checks across goal and chat edits; still
  invalidate them after consent or credential changes.
- Add an isolated live check for a mixed goal-creation, completion and checkpoint
  request followed by confirmation, including unanswered retry history.

## 0.36.2 — provider connection and chat reliability

- Refresh provider check results in Settings so successful tests remain connected.
- Show expired checks as needing a new check, rather than a broken key.
- Allow up to 60 seconds for interactive AI reasoning and do not automatically
  resubmit timed-out generation. Keep authentication errors distinct from timeouts.

## 0.36.1 — goal-bound coaching conversations

- Route messages in a coaching chat through its saved session and selected goal,
  including after an external blocker. Keep that context until explicit exit.
- Start a dedicated chat, save coaching questions and text replies in its transcript,
  and replace the separate questionnaire with pause, resume and exit controls.
- Interpret explicit text answers through the existing validated workflow. Keep
  micro-action reports separate from goal completion and require opt-in result memory.
- Deduplicate turns and reject stale provider results after privacy, goal or session
  changes. Preserve Windows x64 and Apple Silicon macOS signing and updater trust.

## 0.36.0 — Tiny Habits coaching

- Add explicit goal/chat entry, persistent session controls and a real provider
  gateway for 10–60 second micro-actions. Preserve ordinary chat and parent-goal state.
- Recheck privacy and revisions around provider calls; keep pause/stop available
  during generation. Include coaching-only chats in conversation lifecycle controls.
- Add an explicit real-provider diagnostic using a synthetic in-memory coaching
  session. Repair the acceptance map to reference the current memory-deletion test.

## 0.34.0 — one typeface, calmer Settings, Goals and Progress

- One typeface everywhere: Zen Maru Gothic for titles, the wordmark and all text; the serif is gone. Four sizes (34/18/14/11) and two weights, documented in docs/TYPOGRAPHY.md. Text is ink or muted; green marks only links and the selected option of a control.
- Dark theme "Paper at night": warm charcoal surfaces with green kept for the accent, tags and the mascot.
- Settings: no subtitle or repeated headings; quiet mode and snoozing live only in the sidebar; the Appearance tab is gone (the theme switch stays in the sidebar). Companion options are one line each and on by default; one "Remind me to move every 45 minutes" toggle drives both reminders. macOS-sized switches and tighter rows.
- Companion colour: "Choose the color of your companion" shows six animated states (working, listening, thinking, done with confetti, resting with a breathing mouth, stretching) and a Custom colour wheel; any colour is kept. Marks beside the mascot follow the theme text colour and are no longer clipped.
- Goals page: the field searches saved goals or adds a new one, like Today; rows match Today with no separators; Active and Archive page ten rows at a time with the shared Pager; Import sits on the heading line.
- Today: clicking the blank part of a row opens its steps; step ideas while typing continue the typed text and say when they are loading, empty or unavailable; the goal menu is left-aligned.
- Progress: Details is "Tracked activity" with ten-row pages and an empty state; the period controls are one height; the focused-time figure uses the heading size.
- Chats: heading level with the chat pane, plus button in the search field, plain title, flat composer icons, solid row menu, list items in ink.
- Local macOS test builds must be internally signed and versioned above the latest release (AGENTS.md); an ad-hoc build cannot read saved provider keys.

## 0.33.2 — neutral dark theme

- Replace green-tinted dark backgrounds, panels, inputs, dividers and text with neutral charcoal and gray tones. Disable the tinted grain texture in dark mode.
- Preserve green accents and explicit service status colors. Light mode and the selected companion color are unchanged.
- Shared theme tokens apply consistently on Windows x64 and Apple Silicon macOS.

## 0.33.1 — startup connection checks and immediate companion color

- Check Nebius authentication/model access and Tavily authentication automatically when the workspace starts and when connectivity returns. Show a neutral checking indicator until each provider finishes; retain explicit refresh.
- Avoid duplicate startup probes under React StrictMode. Background metadata polling does not make provider requests, and simulated providers stay simulated.
- Save the selected companion color immediately, independently of other settings drafts. Notify both app windows without waiting for dashboard polling; validate supported colors in Rust.
- Keep existing credential access, trusted updater signatures and macOS signing identity unchanged.

## 0.33.0 — compact companion controls and transparent input

- The companion's toolbar uses its selected color, smaller icons and a subtle chat/microphone divider.
- After 500 ms outside the character and controls, the toolbar collapses into a nearby 24 × 4 px capsule. Hovering or keyboard focus reveals it; open chats keep controls accessible.
- The capsule expands downward from a fixed edge without moving the character; reduced-motion preferences disable transitions.
- Click targets follow the painted character silhouette. The native companion window passes transparent margins through to the application below while preserving chat, cards and drag interactions.
- Includes the Today, priorities, inline editing and automatic checkpoint changes from 0.32.2. Windows x64 and Apple Silicon macOS share one release; trusted update signers and the credential helper are unchanged.

## 0.32.2 — calmer Today, in-place editing, priorities

- Goal rows: the ⋯ button is the only control on the left (click for actions, hold to drag); its menu is down to Goal details, Remove from Today and Delete. Steps get a hover × instead of a menu.
- Goal details is a compact editor (area, done-when, deadline, estimated time, description); title, steps and priority are edited in place. The deadline uses a calendar with a Now button and a time you can type ("13:00" or "1:30 pm"); without a time it counts as the whole day.
- Each row shows its deadline (Today goals without one are due today) and the expected time; click it to change.
- The day summary now leads with focused time ("3 h 40 min of focused work today"), shows goal and step counts on the right, and opens to list finished goals and steps with the time spent on each.
- Checkpoints are automatic: when sumi sees you move to another goal, the one you left is parked with where you were (app, window, next open step) and resumed when you come back. Park and Resume in the chat still work.
- Today is one plain list again: no "In focus" and "Later today" lanes, no focus bar, Focus now, Park or Checkpoint buttons. sumi works out what you are doing by itself.
- Every goal row has a priority tag (Urgent, High, Medium, Low or none) with number-key shortcuts; Today sorts by priority, then by your drag order. Urgent is a new level.
- Goal titles and steps are edited in place like a document: click the text, type, press Enter or click away to save, Escape to undo. Emptying the text deletes the goal or step.
- Fix "Your goal or sharing choices changed" appearing on a chat answer when a goal was edited or the focus changed while sumi was answering; only real sharing, privacy, tracking or provider changes discard an answer now.
- One field on Today now finds or adds a goal: typing filters your saved goals (titles, descriptions and steps), Enter adds the highlighted match to Today or creates a new goal, and an empty field shows five recent goals.
- Clicking the empty field lists every saved goal (unfinished ones first) in a scrollable dropdown; there is no separate "Choose goals" button any more.
- Removed the duplicated "Goals", "Choose goals for Today" and "Pick up where you left off" headings and the helper line; unfinished goals from earlier days are marked in the suggestions and can be dismissed there.

## 0.29.0

- Constrain structured agent operations and render authoritative Rust action/read receipts.
- Add live AI monitoring and isolated action diagnostics without sensitive content.
- Restore local foreground-window history independent of goal selection and show observed activity in Progress.
- Separate Today focus from the reorderable queue and follow saved goal order.


## 0.28.1 — sumi look

- Adopt the sumi site design across the app: Shippori Mincho for headings and the brand, Zen Maru Gothic for text, the moss-green accent on a sky-blue and white light theme and a deep-green dark theme, pill buttons and a subtle paper grain.
- The app is now called sumi in the window, Dock, menu bar and all texts, with a new app icon (the mochi on sky blue). The bundle file name stays "Desktop Buddy.app" so updates keep working.
- The companion is now the square kaku-mochi mascot from the site (no animal variants), with sleepy, alert and happy eyes for its states and three colors (Matcha, Mist, Cloud); saved colors migrate to the nearest one.

## 0.27.5 — goal time on macOS counts again

- Count focused time for the goal you selected, including meetings, mail and other windows that name no goal. Since 0.27.0 only windows whose title matched the goal's keywords were counted, so Progress stayed empty.
- Activity whose window clearly belongs to another goal, or is ambiguous between goals, still stays out of the selected goal's time.

## 0.26.1 — chat window, voice and titles

- Make the mascot chat larger with full history, a close button and the mascot below the card.
- Stop voice recording on a pause, send spoken messages in the mascot chat and keep quiet speech.
- Send with Enter in app chats, remove the goal-context block and subtitle, and title chats by topic.
- Redesign Chats with working search (titles and message text), a header menu and a hover menu with Delete on each chat.
- Save every question at once and keep each answer in the chat it was asked in; empty chats are removed automatically.
- Format answers like Claude (paragraphs, lists, copyable code blocks, a Sources list, time and a Copy button) and make Buddy write out the actual answer, including names of sites and tools.

## 0.26.0 — Progress overview and per-step time

- Redesign Progress as an overview and a single-goal view with a stacked chart and period navigation.
- Record relevant activity time per step and show minutes for each finished step; Day shows an hourly chart and a calendar picker chooses the period.
- Buddy chat can add goals and steps, rename, edit, complete and delete them (a deleted goal can be restored with its time for 30 days), and no longer searches the web for questions about the app.
- Today holds any number of goals.
- One page layout, callout style for warnings and redrawn switches across the app.

## 0.25.1 — macOS permission and mascot fixes

- Make Check permission respond, auto-refresh while blocked, and add Reset and ask again for stale Accessibility entries.
- Keep the mascot window on all Spaces and above full-screen apps on macOS.

## 0.24.1 — dark theme, feedback and macOS signing path

- Improve dark theme contrast for buttons and form controls.
- Use a single email-draft feedback flow with clear success and error states.
- Prepare optional Developer ID signing and verification in the macOS build script.

## 0.24.0 — goals, steps and daily summary redesign

- Redesign goals and steps in one shared Notion-style layout, with press-and-hold reordering, hover actions and a completion circle.
- Add a typing-first step composer with an Ideas menu and AI completions.
- Complete goals and steps together, and capitalize new goals and steps.
- Replace the daily summary with a single concise line and add Quiet mode to Today.

## 0.23.2 — coordinated Windows and macOS updates

- Publish Windows and both macOS architectures together after signed artifact and channel verification.
- Use the enrolled Windows CI signer while retaining the primary-signed 0.23.1 transition and older compatibility bridges.
- Reject replacement of published releases and remove the separate Windows-only publication path.

## 0.23.1 — third-developer Windows signing transition

- Enroll `windows-ci` as an additional Windows updater signer in trust epoch 3, keeping `primary` and `developer2` trusted.
- Publish an epoch-2 bridge signed by an existing signer so installed Windows clients can safely trust future CI updates.
- Preserve the separate macOS signing key, trust epoch and update endpoint.
- Include the shared compact Buddy, local voice and workspace changes from 0.23.0.

## 0.22.1 — update indicator and permission guidance

- Highlight an available update in blue, with readable text in both themes.
- Point paused progress checks to the single Activity & Privacy switch.
- Show macOS permission recovery instructions only when access is missing, including renewal after an ad-hoc signed update.

## 0.22.0 — goal monitoring and macOS updates

- Enable local activity tracking and AI completion checks together through one explicit onboarding/settings choice.
- Use persisted AI assistance consent for progress detection and offer confirmed goal completion with exact visible evidence.
- Produce signed Apple Silicon update archives, verify manifests and publish both platform bundles through one GitHub release workflow. macOS uses an independent trusted signing channel; Windows bridges remain intact.
- Show activity tracking status and a direct setup action on Today and Progress, including paused collection and permission errors.
- Describe the selected goal without implying collection is running. Keep completion explicit through goal and step actions.
- Show observed activity in daily summaries and distinguish unmatched work from days without records.

## 0.21.0 — macOS support

- Add a shared macOS application with an Apple Silicon build (with an optional universal build) targeting macOS 14.5 and later.
- Add native foreground/idle tracking, bounded Accessibility context, Keychain credentials, app inventory, login startup and Command + Option selection shortcuts.
- Adapt role app rules, permission controls, voice input and platform wording for macOS. On-device speech requires supported installed language assets.
- Preserve the Windows installer/updater and add macOS CI artifacts in the same repository. Initial macOS installation is manual and locally ad-hoc signed; Developer ID notarization and exact 14.5 hardware validation remain separate requirements.


## 0.18.0

- Connect Goal Analyzer to Nebius with conditional Tavily research, sourced prerequisites, relevant resources and conservative time/difficulty estimates.
- Add explicit AI/context/web permissions, direct Ask Buddy, and visible Tavily key/connection controls.
- Replace the mini chat modes with a shared persistent conversation, attachments, editable voice input, source explanations and retained local inbox actions.
- Preserve tracking consent and pause choices across goal switches; expose collection errors and activity segments, and show observed time separately from the manual timer.
- Offer bounded midday help for overdue goals or measured overruns against user estimates; keep task changes explicitly confirmed.
- Improve local Russian/English phrase selection, reject uncertain recognition and show installed speech languages with retry/paste fallbacks.
- Preserve signing trust epoch 2 and the published 0.17.1 update bridge.

## 0.17.1

- Preserve all 0.17.0 Goals / Today features while enrolling the owner's second Windows computer as update signer `developer2`.
- Advance signing trust to epoch 2, retaining the primary key and verified update paths for older installations.
- Handle Tauri key-generation warnings in Windows PowerShell without aborting setup or exposing captured signing output.

## 0.17.0 — local, unpublished

- Simplify Today around goals with direct entry, inline steps and clear goal actions.
- Add deadlines, priority and descriptions without rewriting existing goals.
- Offer unfinished goals during planning, with persistent per-day dismissal.
- Add goals by planning date and minute/hour progress formatting.
- Receive reviewed selected-text drafts through Windows accessibility and a Chrome/Edge context-menu extension.
- Provide Developer 2 with goal analysis callbacks and suggestion/warning/resource rendering.

## 0.16.1

- Initialize the database and managed command state before creating either webview, preventing startup requests from reporting unmanaged state.
- Clear recovered dashboard and goal-loading errors without dismissing failed user actions.

## 0.16.0

- Check for and apply signed updates at startup before workspace editing; keep the installed version usable if GitHub or installation fails.
- Keep background update checks as notifications, and check again when connectivity returns.
- Pin updater installations to the running executable's folder and block downgrades in newly generated Windows installers.

## 0.15.1 — local, unpublished

- Remove decorative section frames and the repeated Progress heading; use spacing and a date-range toolbar to establish hierarchy.
- Keep selected days visible with theme-aware accents, readable time totals and a horizontally scrollable seven-day strip on narrow windows.
- Simplify Today sections, align page headings with the workspace and retain visible input and keyboard focus affordances.
- Correct the empty preview's dates and replace undefined surface colors with the existing light/dark theme tokens.

## 0.15.0

- Combine the core workspace with the 0.14.0 desktop companion and its existing consent controls.
- Add parallel goals and areas, editable steps, a one-to-three-goal Today plan and optional no-plan days.
- Add reviewed text, image, PDF and local Windows voice imports; AI proposals require explicit sharing and confirmation.
- Add a manual focus timer, day/week progress and a positive daily summary without requiring screen tracking.
- Simplify onboarding to a name, intentions, first goals and optional tracking; keep provider and companion controls in Settings.
- Share plan revisions, completion history and goal transitions between the workspace and mini chat.

## 0.14.0 — unpublished

- Add single-click companion mini chat, local quick tasks/inbox, explicit AI help, Windows voice typing and a selected-text shortcut prototype.
- Replace the Workspace label with native context actions and a persistent Open/Show/Hide/Tracking/Settings/Quit tray menu.
- Add unified activity events, opt-in visible-text task/completion proposals and confirmation with concurrent-plan guards.
- Add gentle daily check-ins, persisted cooldown/deduplication, a shared three-per-day prompt budget and meeting/full-screen/quiet-time suppression.
- Add calm companion states and an optional stretch invitation; keep light/dark and reduced-motion support.
- Document the Developer 1 boundary in `docs/COMPANION_MVP.md`.

## 0.13.0

- Separate personal Windows builds without signing credentials from official signed releases.
- Add local protected developer key setup and reviewed enrollment of multiple trusted update signers.
- Preserve verified update paths for existing installations through pinned compatibility channels.
- Include the avatar hover and compact-window fixes prepared for 0.12.1.

## 0.12.1

- Keep the desktop avatar background transparent on hover and use a muted keyboard focus outline.
- Fit the avatar and Workspace button inside the compact companion window so ears remain fully visible.

## 0.10.0

- Refresh the Windows workspace with a compact sidebar, neutral dark surfaces, consistent typography and dedicated Focus, Activity, Resources, Goal history and Settings views.
- Group settings with progressive disclosure and accessible switches. Preserve drafts when navigating between workspace views.
- Add real GitHub Release checks and a blue Update available indicator for newer published Windows versions. Separate unpublished releases and connection errors from successful checks.
- Download and verify signed installers only after an explicit Update & restart action. Keep local data and saved provider credentials across upgrades.
- Add a signed release workflow, version-pinned updater manifests and regression coverage for concurrency, offline checks and failed-install retries. See UPDATES.md for signing-key setup.

## 0.9.0

- Add resumable first-run setup: character/color, role/name/required email with a clearly labeled draft privacy notice, and a goal using the existing model. Ask about activity tracking afterward, with an option to continue paused.
- Share eight role presets between React and Rust, including Figma in Design. Apply starter app rules per goal while preserving manual overrides and explicit unclassification.
- Extend the existing companion with cat, dog, seal and bird appearances, state-driven expressions, color selection and complete visibility control.
- Consolidate profile, app rules, hours, startup and notification controls under the Settings gear. Enable installed Windows autostart by default after onboarding; development builds never register for startup.
- Add local star/text feedback in Settings and after goal completion, with a repository boundary for future feedback storage. Keep identity and feedback out of provider requests.
- Add profile validation, onboarding persistence, migration, preset and feedback regression coverage. See `docs/ONBOARDING.md` for affected files and native QA.

Known limits: the privacy notice is a placeholder awaiting a reviewed final policy. There is no remote registration/feedback service or voice recording. Windows sign-in startup and native companion interactions still need the manual QA checklist.

## 0.8.0

- Add foreground-browser title/domain metadata through a bounded Windows UI Automation adapter; retain only normalized hostnames and fall back to titles.
- Add deterministic focused, paused, drifting and resumed states, plus goal-completed events for the avatar API. Default idle threshold is five minutes.
- Suppress all automatic productivity notifications outside configurable local working hours (09:00–18:00 by default) and during confirmed system media playback.
- Extend observed usage tracking with disjoint app/site/tab and idle intervals, goal-completion timestamps and a typed goal-history API. Keep recommendations goal-scoped and persistence-safe.
- Add backward-compatible SQLite migrations and regression tests for transitions, media/hours suppression, overlap rejection, completion/restart, retention and legacy data.

Known limits: address-bar availability depends on browser/version/language; only domain/title metadata identifies pages. System media playback is confirmed only for exact executable identities. Unobserved transitions, sleep, app downtime and manual-pause durations are not inferred. See `docs/TRACKING.md` for Windows QA.

## 0.7.0

- Prioritize the saved current step and completion criterion in both recommendation query planning and resource selection.
- Use up to ten rated titles and ten recently offered titles from the current goal in both stages; query goal-scoped history directly instead of filtering the latest 100 global recommendations.
- Allow query planning and resource selection to abstain. Require a short, concrete next action for each selected resource and retain it in the existing recommendation reason.
- Deduplicate retained history and each result batch using URL identities without fragments or known tracking parameters (`utm_*`, `gclid`, `fbclid`, `msclkid`). Keep functional parameters and original link destinations intact; support legacy saved URLs without rewriting history.
- Update privacy disclosures and the local preview to include the recommendation memory actually sent to Nebius. Existing consent, cooldown and stale-response checks remain in place.

Known limits: semantic relevance and avoidance of similar content depend on the model and search snippets, not full-page verification. Different aliases/redirects and unknown tracking parameters may still represent duplicates. Deduplication is per goal and only covers retained history; retention/deletion can allow a resource to appear again. Live provider quality remains a manual check.

## 0.6.0

- Add per-goal Work, Distraction and Neutral process rules. Explicit rules override automatic AI focus judgments; unclassified apps retain the consented AI path.
- Add opt-in, provider-free distraction reminders after two active minutes in the same window. Explain the user's rule in each reminder; do not block apps or inspect browser domains.
- Apply a persisted, shared daily card limit (default 8) and minimum interval (default 15 minutes) to reminders, AI focus cards and recommendations. Quiet controls still apply; manual focus checks remain available in the workspace.
- Add sampled daily foreground time across goals and for the current goal, plus saved checklist progress. Do not infer past usage from legacy cumulative activity rows.
- Skip unobserved gaps, app/goal switches, pauses, excluded apps, Buddy itself and detected idle periods. Split samples at local midnight. Demo data stays in its separate database.
- Include daily usage in retention and confirmed history deletion. Keep active-goal rules and card-budget settings on history deletion.

Known limits: time is approximate, not whole-device usage; passive reading without input becomes idle after 60 seconds. Browser tabs share one process rule. Daily buckets retain their collection-time local date when the time zone changes. Card slots are reserved before native display, so a failed native show can conservatively consume a slot. No live provider or installed/native end-to-end verification is implied by automated tests.

## 0.5.0

- Add editable completion criteria, up to 20 ordered checklist steps, and one current unfinished step per goal.
- Save plans locally with revision checks; preserve existing goals on upgrade.
- Defer the previous goal when starting another. Explicit completion requires confirmation; completion, deferral and resumption leave tracking paused.
- Include the saved criterion and current step in consented AI focus and recommendation context, including the local privacy preview.
- Offer optional Nebius goal refinement without activity history. Proposals never save automatically: replacing a checklist requires confirmation, then an explicit save of the editable draft.
- Keep deferred plans during automatic retention; explicit history deletion removes inactive goals and their plans, while preserving the active plan.

Known limits: completed statuses from older versions remain unchanged and may reflect the old automatic-completion behavior. No deadlines, project hierarchy, streaks, or automatic outcome verification. Native end-to-end interaction and live refinement quality require manual validation.

## 0.4.0

- Expose 5/15/30/60-minute automatic recommendation intervals in Settings, separately from focus checks.
- Show the latest 100 retained recommendations with their goals, timestamps, and reversible helpful/not-helpful ratings.
- Add a one-hour Not now action to floating cards and explicit snooze/resume controls in the workspace. Snooze preserves tracking and DND state.
- Retain the 0.3.1 worker-dispatch fix and add regression coverage for rating updates, interval changes, and snooze suppression.

## 0.3.1

- Dispatch IPC commands outside the native event thread to prevent a lock inversion between dashboard polling and Buddy window updates.
- Add a regression guard covering every registered command module, including settings and privacy commands.
- Preserve existing data, consent settings, and provider behavior; no new tracking capabilities.

The deadlock path is identified in code. Reproducing the reported Google Meet screen-sharing incident and verifying the fix in that native workflow remain manual checks.

## 0.3.0

- Add a local preview of the minimized context used by AI requests.
- Add excluded applications, optional 7/30/90-day retention, and confirmed local-history deletion that pauses tracking while preserving the active goal and provider keys.
- Add explicit provider connection checks without sending goals or activity, and safe actionable HTTP errors.
- Add tray actions, close-to-tray behavior, persisted companion position, and scheduling foundations from the ongoing desktop-reliability work.
- Add model-assisted resource selection and stored recommendation records; history, rating, and frequency controls are not yet exposed in the UI.

Known limits: native tray/monitor interactions, live provider access, and install-over-existing-data still need manual verification. History deletion is logical deletion, not secure erasure, and cannot remove data already sent to providers. Automatic updates and installer signing are not implemented.

## 0.2.0

- Move Buddy into a transparent, always-on-top, draggable desktop window.
- Add persistent intermittent/always-visible display preferences and CSS animation.
- Add separately consented Nebius-generated Tavily searches, 15-minute pacing, per-goal URL deduplication, and stale-result rejection.
- Add expiring resource cards, pause/DND suppression, and a return-to-workspace action.
- Synchronize release versions and enforce consistency during frontend builds.
- Add regression tests for preferences, scheduling, consent changes, and companion rendering.

Known limits: position persists only during the current run; closing the workspace exits the app; native overlay interactions and live recommendation quality still need manual verification.

## 0.1.0 — prototype baseline

- Add a Windows-first Tauri/React focus dashboard and Win32 foreground activity collection.
- Persist goals, activity, decisions, and feedback in SQLite.
- Integrate Nebius focus analysis with structured responses and manual Tavily search.
- Add provider Settings with Windows Credential Manager storage.
- Add a separate focus-nudge popup, explicit AI consent, pause, and DND.
- Add simulated activity and clearly labeled mock AI, tests, and Windows CI/NSIS packaging.

## History note

Development started before the first Git commit. The repository begins with a single commit of version 0.2.0. The 0.1.0 section documents earlier prototype work, including provider Settings and response-format fixes; it does not represent a separate Git commit or tag.
