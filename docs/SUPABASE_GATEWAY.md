# Built-in AI and personal provider connections

The installed app selects **Built-in AI** by default. Everyone uses the same registration-free flow: install, complete onboarding and use the app. There are no invitations, access codes or user tiers. **Settings → AI → Services → AI connection** switches to **Use my API keys** without changing or removing saved credentials.

## Routing

All generation, coaching, goal actions, memory-assisted dialogue, screen understanding, background observations, import, web research and provider checks use the selected connection. Built-in generation uses the Claude Messages API with `claude-opus-5-5` for primary work, responses, semantic verification and image descriptions. The server allowlist is explicit; personal connections retain their existing model/endpoint controls.

Built-in requests go from the Rust backend to `buddy-ai` in project `qrspcvglmmrputcltgxr`, then to fixed Claude or Tavily HTTPS endpoints. No route returns an upstream key. The public `server/supabase/public.json` contains only the project URL and public `anon` client key; it is not a provider key or a Supabase service-role key.

An anonymous Supabase Auth session starts without user input. The app keeps its access/refresh session in a private local file, refreshes it automatically and checks the connection generation before sending/accepting responses. Provider credentials continue to use the existing OS vault, and the immutable macOS helper is unchanged. Switching modes invalidates old health receipts and pending-response authority; the sidebar checks the newly selected connection. A failed built-in request does not spend a user's personal-provider balance through silent fallback.

## Server boundary

- Platform JWT verification remains enabled. The function independently verifies each user token with this project's Auth service.
- Only four operations exist: Claude messages and a single-model health read (`/anthropic/v1/messages`, `/anthropic/v1/models/<allowed model>`) plus Tavily search/usage. Arbitrary URLs, remote image URLs, streaming, tools and unlisted models are rejected. The gateway adds the `x-api-key`, `anthropic-version` and `anthropic-beta` headers upstream.
- Provider and service-role credentials exist only in Edge Function environment variables. Success health responses expose model IDs or a configuration boolean; upstream error bodies and account usage details are removed.
- Quota tables have RLS enabled and no grants for `anon` or `authenticated`. Only `service_role` can reserve usage. An advisory transaction lock makes global/user/minute reservations atomic across isolates.
- Counters store anonymous IDs, route counters, conservative reservation units and timestamps only. Prompts, screenshots, goals, search terms and model responses are not persisted by this gateway. Counter rows older than two days are pruned during the next successful gateway reservation. Supabase also retains the technical anonymous Auth records needed for automatic sessions.
- Default policy: 3,000 AI / 300 search calls globally per UTC day; 1,000 AI / 100 searches per anonymous session; 30 total calls per session per minute. A global 30,000,000-unit daily reservation ceiling also bounds use. Text bytes, output-token budget weighted by eight, and image allowances contribute to units; **units are not a currency price**. Failed/timed-out reservations are not refunded. The gateway never retries a provider call. The existing client can try one fallback model for an interactive proposal; that additional request is counted separately and never replays an executed action. These are uniform anti-abuse limits, not feature tiers.
- `buddy_ai_policy.enabled` is an immediate server kill switch. Limits can be changed by the owner without shipping a new app. The endpoint/public client key can be extracted from an installer, so authentication alone cannot stop determined misuse. Atomic global limits bound exposure; upstream provider spending caps should also be configured by the owner.

## Deployment and verification

1. Verify the exact project and organization before deployment. Use the owner's CLI profile; preserve other developers' sessions. Keep access tokens outside Git and command output.
2. Save dedicated `ANTHROPIC_API_KEY` and `TAVILY_API_KEY` in Edge Secrets. Apply `server/supabase/ai-gateway.sql`; its initial policy is disabled.
3. Enable anonymous sign-ins in this project's Auth settings. The app does not collect a name/email or show a login form for this technical session. Keep Auth's built-in abuse rate limits.
4. Deploy only the reviewed function: `supabase functions deploy buddy-ai --use-api --project-ref qrspcvglmmrputcltgxr --workdir server`. Do not use `--prune` or disable JWT verification to clear an unexplained failure.
5. Run `npm run test:gateway`. Verify quota rejection and denied public table/RPC access. Enable the policy only after these checks, then test actual model generation, Tavily search, connection switching and a returning session. No private activity is needed for these checks.
6. Validate macOS and Windows, review the single additional Settings row, then publish one new synchronized version for both systems. Never replace the delivered 0.48.1 assets with different code.

Deployment does not enable profile collection or upload existing Feedback/history. Those remain separate product work.

## Focused live checks

The signed app's opt-in `--check-hero` rehearsal uses disposable synthetic data and real providers. `--only-hero-resume` checks Resume/focus/checkpoint only; `--only-hero-tail` checks Resume, a grounded summary and database-reopen recall with deterministic fixture setup. Use `BUDDY_AI_CONNECTION_MODE=built_in` or `personal` only for these `--check-*` commands. These checks never run on ordinary app startup or in automated mock-provider suites. `BUDDY_DIALOGUE_TRACE=true` is restricted to the supported synthetic CLI checks; it can print their request/response JSON, never provider headers, credentials or provider reasoning content.

## Daily operational availability

The [availability workflow](AI_SERVICE_AVAILABILITY.md) exercises anonymous sign-in and the existing two health routes daily, with sanitized output and no generation/search. Provider secrets stay in Supabase. Daily activity is not a guaranteed replacement for a paid plan's no-inactivity-pause policy.
