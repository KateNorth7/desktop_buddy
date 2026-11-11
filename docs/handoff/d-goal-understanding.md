# Goal understanding, phase 1 hardening

Branch `goal-understanding/d-hardening`, on top of `goal-understanding/c-decisions`.

## What changed

- `decisions.rs`: `Decision.plan_revision` (optional). A finding made against an older plan revision is rejected as `plan_changed` and journalled as `evidence_expired`; attribution is exempt. `completed_elsewhere` puts a completion applied outside the path into the shared cooldown. Semantic observation proposals are journalled as `completion_proposed`.
- `episode_review.rs`: each job records every goal's plan revision; completion proposals carry it, so an edit while the model is answering drops the card.
- `semantic_observation.rs`: completion, new goal and new step proposals now go through `submit_at` (shared dedupe, cooldowns, consent and staleness checks) and are delivered at once with `companion::deliver_now`. The verified automatic completion keeps its own receipt path (`automatically_complete_outcome`) and then marks the shared cooldown. The undo fallback is a shared proposal too.
- `goal_understanding.rs`: the request includes up to six chat messages linked to the goal from the last 30 days (300 characters each, provenance `chat`).
- `test-data/episode-review-benchmark.json` (27 cases, 21 near misses that must never become a card) with `episode_review/benchmark.rs`: an offline test of the data and the app's thresholds, and the opt-in `episode_review_live_benchmark`.
- `npm run bench:live` (`scripts/live-benchmarks.mjs`) runs both real-model benchmarks and prints pass/fail, false cards, misses, errors and latency; the full log goes to `.tools/quality/`. The key comes from `ANTHROPIC_API_KEY` or the ignored file `.tools/anthropic-benchmark-key` in the primary checkout. The macOS vault cannot be read by scripts (the signed helper refuses other callers).

## Not done, for a decision

- A clearly achieved goal with a stale open step is still not proposed (existing rule).
- A chat message added after a goal was understood does not refresh the understanding (only goal edits do).

## Evidence

- `cargo test --locked` 831 passed / 7 ignored; Clippy `-D warnings` and `cargo fmt --check` clean. New: plan revision rejection, an edit during the review, a locked session ending the episode, the semantic path sharing cooldowns with the review, chat context, the benchmark data test.
- `npm test` 349 passed, `npm run lint` and `npm run build` clean. Gateway unchanged.

## Not verified

- Real-model results of both benchmarks (no Claude key on this Mac), Windows build (CI), live delivery.

## On-device text recognition (follow-up commit)

- `native/macos.m` `buddy_recognize_text` (Apple Vision, accurate level, automatic language) and `vision/windows_text.rs` (Windows.Media.Ocr with the user's profile languages) read one captured window image in reading order. `vision::recognize_window` captures the foreground window with the existing guarded capture and recognizes it; the image is neither kept nor sent.
- `companion/screen_context.rs`: when interface text has fewer than 80 letters or digits (messengers, canvases, remote screens) and Use screenshots is on, the recognized text is redacted and appended, then flows through the same cache, review evidence, fast signals and episode review as interface text.
- Settings text, the Privacy preview and `docs/privacy.html` describe this.
- Tests: Vision on the existing fixture (macOS), Windows OCR on the same fixture (Windows CI only), consent and thin-text gates. `cargo test` 833 passed / 7 ignored on macOS.
- Not verified: Windows compile and OCR (CI), visual baselines for Settings (Windows CI), live speed on a signed build.

## Live run on a signed local build (11 November)

An internally signed 0.51.0 build of this branch ran on the Mac with a test goal ("Тест: написать Анне письмо-благодарность после собеседования"), a draft in TextEdit, the fixture image in Preview and unrelated work in Calculator. It found four defects, all fixed:

- **Crash loop on launch** (`12b1b64`): the collector walked sumi's own window through Accessibility from a background thread and WebKit terminated the process. `focusedWindow` now returns nothing for sumi's own process. The code dates from 0.50.0, so the published 0.51.0 is likely affected on this macOS.
- **Recognition hang** (`7d1913d`): Vision text recognition waited forever on the Neural Engine in the signed app and held the AI lane, so nothing reached a goal. Recognition now runs on the CPU, gives up after 4 s and never starts while a stuck call is still running.
- **Goal understanding never ran**: its tick only tried the busy AI lane and also needed the lane to start the 20 s settling period. A due goal now waits for the lane up to 45 s (one waiter, like goal matching); settling runs without the lane; a missed wait is journalled as `skipped_lane_busy`.
- **Goal understanding always failed** (`provider_error`): a nullable enum (`kind`, also `difficulty` in goal analysis) kept `null` inside the string variant after `claude::sanitize` split the type. The typed variant now drops `null` from `enum`/`const`.

Results after the fixes: TextEdit 119 s and Preview 98 s attributed to the test goal (matching confidence 0.93 and 0.8; the Preview text came from recognition), Calculator 125 s correctly unassigned (`answered_no_goal`, 0.15); the goal's understanding was saved within 25 s of launch (`send_email`, uncertainty 0.2, provenance goal text, recent work, personal memory). No crash reports. `cargo test --locked` 835 passed / 7 ignored, Clippy `-D warnings` and `cargo fmt --check` clean.

Still not verified live: automatic completion (no real email was sent), Windows runtime. The outcome check still skips about 100 turns per 5 minutes on the busy lane.
