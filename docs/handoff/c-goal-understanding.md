# Prompt C — one decision path, episode review, quiet-mode receipts (10 November 2026)

Branch `goal-understanding/c-decisions`, on top of `goal-understanding/b-evidence` (steps run one after another).

## What changed

- `decisions.rs`: `submit_at` is the one path from a finding to an action. It checks confidence bounds, a remembered dedupe key (24 hours), interface consent (screen task detection plus AI and goal sharing; attribution excepted), that the goal is open and the step exists and is not done, and per-goal (30 minutes) and per-text (4 hours) cooldowns shared by every analyser. It applies only existing tools: the goal or step completion card (with a short factual reason, for example "Looks like you sent your interview availability. “…”"), automatic completion through `automatically_complete` with its receipt card and durable Undo (a disabled setting turns it into the card; an undone plan is never repeated), the new goal and new step cards (never for an already listed title or step), and `assign_pending_context`, which moves only pending, automatically recorded intervals (assigned, outside and manual time never moves, nothing is counted twice). Journal codes: `outcome_probable`, `completion_proposed`, `completion_applied`, `automatic_completion_disabled`, `proposal_queued`, `proposal_deduplicated`, `evidence_expired`.
- `completion_signals.rs`: fast-path closes and unlinked offers, and the time-based episode-end offer, now go through `submit_at`; its own cooldown map is gone. The time-based offer runs only when the model review is unavailable (`episode_review::reviews_episodes`).
- `episode_review.rs`: one bounded request on the configured model when an episode ends, when work on an attributed goal stopped 150 s ago while other activity continued, or every 10 minutes of a long episode; at most one per minute, budget through `reserve_secondary`, the lane is only tried, 60 s timeout, no lock across the call, each episode key reviewed once. Input: up to 40 windows (app, minimized title, site, seconds, result code, matched goal) with sealed excerpts for at most six windows that carry a result or the most time, and up to 20 open goals with steps, user criterion and stored understanding. Output per window (goal or none), per goal or step (achieved, not achieved, unclear, with evidence indexes and a phrase completing "Looks like you …"), and an optional new goal. Validation rejects any unknown window, goal or step. Application: pending windows at confidence ≥ 0.8 are attributed; achieved results with evidence at ≥ 0.75 become a proposal card (never an automatic completion); a new goal needs ≥ 0.8 and at least three unassigned minutes. Messengers are judged by content, not app category.
- `companion/queue.rs`: the automatic-completion receipt (with Undo) is postponed through Do not disturb and quiet hours instead of being dropped, including one already on screen; other prompts keep the previous behaviour.
- `episode_evidence.rs` (B): windows now carry `period_id` and `last_at`. `activity_review/storage.rs` (B): `assign_pending_context`.
- Shared additive edits: `commands.rs` (two runtime fields), `lib.rs` (module), `agent_runtime.rs` (one spawn).

## Not done, by design or for a decision

- No new clarifying-question card: the existing completion card with a factual reason covers the cases, and asking upfront was rejected by the user. A multi-option card would need new UI and visual baselines (owner review).
- Semantic observation keeps its own verified completion path; it already uses the same mutation methods but is not yet routed through `submit_at` (its dedupe is per card id). Routing it is the next step for one fully shared cooldown.
- The goal card can only close a goal without unfinished steps (existing rule); a clearly achieved goal with a stale open step is not proposed.

## Evidence

- `cargo test --locked` 826 passed / 6 ignored; Clippy `-D warnings` and `cargo fmt --check` clean. New: six decision tests (one offer across analysers, stale goal or step, open steps, consent, automatic completion setting and receipt, duplicate goals and steps, pending-only attribution), five review tests (trigger after goal work stops and only once, short episodes and missing consent, invalid answers, a supported result with reason versus weak ones and an unrelated messenger window, unlisted work), one quiet-mode receipt test; both real-case replays now get their close offer from the scripted review after the finishing window.
- Frontend and gateway unchanged since Prompt B (349 and 13 passed there).

## Not verified

- Real model behaviour of the review prompt (needs the opt-in benchmark and a live run), Windows build (CI only), live delivery of cards.
