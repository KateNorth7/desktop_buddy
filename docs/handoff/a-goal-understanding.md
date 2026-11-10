# Prompt A — goal understanding, safe fast paths, migration (10 November 2026)

Branch `goal-understanding/a-understanding`, on top of the contract commit on `claude/tracking-diagnostics` (0.51.0, Claude Haiku 5.5).

## What changed

- `goal_understanding.rs`: background interpretation of each open goal through the existing `Service`/`request_validated` path with specialist observability. Inputs (only under the consent that already sends open goals to the matcher: AI on, goal sharing on, automatic matching on, tracking on, onboarding complete): title, description, steps, the user's `done_when` (takes precedence), the noticed-work context the goal was created from, up to six recently attributed windows (app, minimized title, site; last four hours) and relevant personal memory through `observation::personal_context`. Conversation context is not included yet.
- Output, strictly validated and bounded: meaning, one to four outcome hypotheses, entities, targets, optional fast kind, uncertainty; provenance codes and an input fingerprint are added locally. The stored row is valid only while the goal's title, description, steps, `done_when` and plan revision are unchanged; any edit makes it stale.
- Scheduling: one goal at a time, Today goals first, then newest; edits settle for 20 s; at most one request per 30 s, so upgrading with many open goals understands them gradually (this is the migration); a failed revision retries after 5 minutes, at most three times; budget through `reserve_secondary`; the shared AI lane is only tried, never waited for; 60 s timeout; the state lock is never held across the call; answers for an edited or closed goal are dropped (`evidence_expired`).
- Journal stage `goal_understanding`: `understanding_saved`, `understanding_uncertain` (uncertainty ≥ 0.6), `skipped_hourly_budget`, provider codes, `evidence_expired`.
- Safe fast paths (`completion_signals.rs`): kind and targets come from the understanding first, the title classifier only as fallback. A result closes a goal only when it connects to the goal's target (the named meeting or person in the meeting title or call screen, the recipient in the sent mail, the company or form on a submitted form page, the requested site itself). No known target: the result is only offered (`outcome_probable`, once per 30 minutes). A contradicting target (another meeting, another recipient) does nothing. A pre-join screen is not joining; a sent or submitted confirmation counts only when it newly appears in that window (an old or quoted one already on screen does not); a form page with an error does not count; search results are not the site. New fast kind: submitted form.
- Shared additive edits: `commands.rs` (runtime field), `agent_runtime.rs` (one spawn next to goal analysis), `tracking_attempts.rs` (stage constant).

## Evidence

- `cargo test --locked` 811 passed / 6 ignored (new: 7 understanding tests, 4 rewritten or new fast-path tests including a Russian goal with English meeting UI); Clippy `-D warnings` and `cargo fmt --check` clean. Frontend unchanged.
- Opt-in real-model benchmark: `src-tauri/test-data/understanding-benchmark.json` (12 varied cases: Russian, Hebrew, German, Spanish, terse and vague goals, user criterion precedence). Run with `SUMI_LIVE_BENCHMARK=1 cargo test --locked --lib -- --ignored understanding_live_benchmark --nocapture`. **Not run yet**: no live accuracy, latency or cost is claimed.

## Not verified / limits

- Real model accuracy, latency and cost; Windows build (CI only); live run in the app.
- The understanding is not yet read by goal matching or the episode-end offer; that belongs to Prompt C through `decisions.rs`.
- Fast paths depend on interface text having been read while the result was visible; when it was not, the result stays unknown or is only offered.
