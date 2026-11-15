# Prompt B — episode evidence, persistence, usage accounting (10 November 2026)

Branch `goal-understanding/b-evidence`, on top of `goal-understanding/a-understanding` (steps run one after another).

## Desktop: evidence across an episode

- No parallel store. `episode_evidence.rs` reads the existing review tables: measured windows from `review_intervals`/`review_contexts`, sealed text and image samples from `review_evidence`. The only schema change is an additive `review_evidence.result_signal` column holding a fixed code (`message_sent`, `form_submitted`, `form_error`, `in_call`, `pre_join`) computed from the redacted text when it is sealed; the text itself stays encrypted.
- `Storage::episode_evidence(range)`: windows of the range, oldest first, with app, minimized title, site, measured seconds, the goal they are attributed to (uncertain time stays `None`), the last step, the newest result code, provenance (`interface_text` or `window_metadata`) and an `excerpt_ref`. `Storage::episode_text(excerpt_ref)` decrypts the newest retained text of that window, bounded to 600 characters. Consent is not persisted across restarts, so `consent_revision` is 0 for stored evidence and consumers recheck current consent before any disclosure.
- `Storage::episode_summaries(range)`: episodes split at gaps over 120 s and at two hours; each summary lists up to 12 windows (place, minutes, result code) in at most 600 characters, the source context keys and the attributed goals. Summaries are computed from stored evidence when read: they never contain screen text and never infer a purpose.
- Lifecycle and limits: everything persists across restarts (it is the review data) and is deleted with the review period after the daily summary, by retention and by Clear local history; reads return at most 200 windows; the existing 64 MB sealed-evidence ceiling still applies. Idle, locked and excluded time is never recorded, so it never appears.
- The shared result phrases moved to `episode_evidence.rs`; `completion_signals.rs` now uses them (no duplicate lists).

## Supabase gateway: per-user usage, cap, spike block, alerts

- `ai-gateway.sql` (idempotent): `buddy_ai_usage_daily` (per user, UTC day, route and model: calls, input/output/cache-write/cache-read tokens, searches, settled micro-dollars; 90 days), `buddy_ai_blocks`, the developers' view `buddy_ai_spend_by_user_day`, `buddy_record_ai_usage` (records, settles the per-user counter, detects a spike and blocks until the end of the UTC day) and `buddy_unblock_ai`. `buddy_reserve_ai` keeps its signature and now also rejects blocked users and enforces `user_microusd_daily` (default $0.40). Spike defaults: today above 3× the 7-day average and above $0.20, or more than 60 calls in 10 minutes. All tables are service-role only.
- `handler.ts`: after settlement it records usage with token counts only, and on a new block sends one Telegram message (user ID, reason, counters, unblock command) when `TELEGRAM_BOT_TOKEN` and `TELEGRAM_ALERT_CHAT_ID` are set; failures never affect the delivered answer.
- `docs/SUPABASE_GATEWAY.md` documents the report query, the cap, spike rules, alerts, the unblock function, the deploy order (SQL before the function) and the Anthropic Console monthly limit as the hard backstop.
- Not deployed. **The SQL was not executed anywhere**: no Postgres is available locally, so its syntax and behaviour are unverified until it runs on a Supabase branch or local stack.

## Evidence

- `cargo test --locked` 815 passed / 6 ignored (new: four episode-evidence tests including a restart from an on-disk database, the 200-window bound and Clear local history); Clippy `-D warnings` and `cargo fmt --check` clean.
- `npm run test:gateway` 13 passed (new: per-user record without content, one alert per new block and none without a bot or for an existing block, a failed record keeps the answer).
- `npm test` 349 passed; `npm run lint` and `npm run build` clean.
- `npm run verify:privacy` reports that the published privacy page differs from the local one; this predates this work (the page is hosted on the original Pages site) and is for integration.

## Not verified

- The SQL migration and the live gateway; Telegram delivery; Windows build (CI only).
- Nothing consumes the episode evidence yet; Prompt C's episode review is the consumer.
