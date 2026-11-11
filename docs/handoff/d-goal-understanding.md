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
