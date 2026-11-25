# Team stats

https://katenorth7.github.io/desktop_buddy/stats/ shows how sumi is used, for the people building it. The page lives on the `gh-pages` branch (`stats/`), is not indexed, loads no third-party code and asks for the team key once per browser. The key is the `STATS_KEY` Edge Secret; the copy that created it is in the ignored `.tools/stats/team-key.txt` of the worktree that set it up. Rotate it with `supabase secrets set STATS_KEY=… --project-ref qrspcvglmmrputcltgxr`.

## Sources

`buddy-stats` (`server/supabase/functions/buddy-stats/`) builds every number server-side and returns aggregates only: no user IDs, IPs or content. The app sends nothing new.

- **Installs and use** come from records Supabase already keeps for the built-in AI connection. An install is an anonymous Auth session that refreshed at least once; the app refreshes it about hourly while it runs, so each refresh stands for an hour in use. The daily availability check signs out (its session and tokens disappear) and test tooling identifies itself as `node`; neither counts. Installs that use only personal API keys are invisible.
- **Recently online and AI sessions** come from the gateway's per-minute counters in `buddy_ai_counters`, kept for 48 hours. A gap over 15 minutes ends an AI session; a gap over 90 minutes between refreshes ends an app session.
- **Downloads** come from GitHub release asset counters. Mac installers (`.dmg`) and Mac updates (`.app.tar.gz`) are separate; Windows updates reuse the installer, so its count includes updates.
- **Website** comes from Google Analytics 4 through the Data API when `GA4_PROPERTY_ID` and `GA4_SERVICE_ACCOUNT` (a service-account JSON with Viewer access to the property) are set. GA4 only sees visitors who accepted the cookie banner.

Per-install rows show only a number (`#n`, by first appearance), status, first and last refresh, days active in 30 days, hours open in 7 days and refresh counts; the anonymous ID never leaves the function.

Data start: installs and use 28 October 2026 (first anonymous session); downloads 6 November 2026 (first fork release, cumulative counters); website 23 November 2026 (GA4 on the site), consented visitors only from 24 November; recently online and AI sessions keep only the last 48 hours.

Not available without app changes: device model, operating system and exact session length. Per-user daily AI usage needs `buddy_ai_usage_daily` from `server/supabase/ai-gateway.sql`, which is not applied in production.

## Deploy

`supabase functions deploy buddy-stats --use-api --project-ref qrspcvglmmrputcltgxr --workdir server`. JWT verification is off for this function; the handler requires the team key, answers CORS only for the Pages origin and local previews, and caches results for five minutes. `npm run test:gateway` covers it.
