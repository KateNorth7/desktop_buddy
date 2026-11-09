# Built-in AI availability

The `Built-in AI availability` GitHub Actions workflow checks the deployed public application route every day at **07:17 UTC**. It also supports manual dispatch. It runs on the repository's default branch after the workflow is published there; an unpushed local file does not schedule anything.

## What one check does

1. Read only the checked-in public Supabase client configuration.
2. Start a disposable anonymous Auth session, through the same public signup path used by a new app installation.
3. Request the Claude model health route for each expected model (`/anthropic/v1/models/<id>`). Verify that the returned model object matches the configured ID.
4. Request the existing Tavily usage health route. The gateway returns only a configuration flag, not upstream account usage or provider secrets.
5. Sign out. Do not persist access or refresh tokens.

Both gateway health operations call the existing database-backed quota reservation function. This creates real API/database activity and prunes old quota counters through the existing policy. No new public RPC, table, provider secret, authentication bypass, installed-app change or invitation flow is introduced. One technical anonymous Auth record can remain per run; it contains the monitor's purpose label, not a person's email/name. The job makes no generation or search requests and uploads only a sanitized report for 14 days.

The workflow has read-only repository permissions and uses the public client key already shipped with the application. It needs no Claude/Tavily/service-role key in GitHub Secrets. Failure reports contain a fixed stage and optional HTTP status, not upstream bodies, account identifiers or exception text. GitHub Actions notification delivery depends on the repository owner's notification settings.

## Availability and inactivity pausing

The project dashboard was observed on 29 October 2026 as **Free**. Supabase states that a Free project can pause after insufficient database activity over seven days. Real daily activity can help, but is **not a no-pause guarantee**. Supabase states that paid projects are not automatically paused for inactivity. This workflow cannot resume an already paused project and its own scheduled execution is subject to GitHub availability and scheduling policy.

- [Supabase project pausing](https://supabase.com/docs/guides/platform/free-project-pausing)
- [GitHub scheduled workflow events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule)

For judging, maintain a functioning gateway and sufficient provider balance/limits through **15 December 2026**, the published end of judging. Check project plan/pausing notices, provider credits, rate limits and key revocation as well as key expiry. A far-future key expiry does not guarantee funded inference or uninterrupted access.

## Manual verification

```bash
node --test server/supabase/availability.verify.mjs
node server/supabase/availability.mjs --report /absolute/path/to/availability-report.json
```

The live command performs the external operations listed above. The unit tests use injected responses and do not contact providers. The public legacy client JWT's expiry is decoded as informational metadata; provider authorization and the Supabase server still validate credentials normally. Short-lived user access tokens are distinct from the public client key: the desktop refreshes saved anonymous sessions automatically.

A passed report establishes authentication, database-backed gateway health and model availability. It does **not** establish successful inference, working native capture, microphone recognition, task extraction or goal completion. Keep the [native demo rehearsal](HACKATHON_DEMO_RUNBOOK.md) separate.
