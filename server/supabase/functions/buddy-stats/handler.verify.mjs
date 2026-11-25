import { test } from "node:test";
import assert from "node:assert/strict";
import {
  createStats,
  keyMatches,
  percentile,
  sessionDurations,
  summarizeReleases,
} from "./handler.ts";

const KEY = "team-key-for-tests-0123456789abcdef";
const env = (values) => (name) => values[name];

test("the team key is required and compared exactly", async () => {
  assert.equal(await keyMatches(KEY, KEY), true);
  assert.equal(await keyMatches(KEY + "x", KEY), false);
  assert.equal(await keyMatches(null, KEY), false);
  assert.equal(await keyMatches("short", "short"), false);
});

test("sessions split on gaps longer than fifteen minutes", () => {
  const durations = sessionDurations(
    new Map([
      ["a", [100, 101, 110, 140, 141]],
      ["b", [500]],
    ]),
  );
  assert.deepEqual(
    durations.sort((x, y) => x - y),
    [1, 2, 11],
  );
  assert.equal(percentile([1, 2, 11], 50), 2);
  assert.equal(percentile([], 50), null);
  assert.deepEqual(
    sessionDurations(new Map([["a", [0, 60, 200]]]), 90),
    [61, 1],
  );
});

test("downloads separate installers from updates", () => {
  const summary = summarizeReleases([
    {
      tag_name: "v0.52.1",
      published_at: "2026-11-17T20:05:46Z",
      draft: false,
      assets: [
        { name: "sumi_0.52.1_aarch64.dmg", download_count: 1 },
        { name: "sumi_0.52.1_aarch64.app.tar.gz", download_count: 4 },
        { name: "sumi_0.52.1_aarch64.app.tar.gz.sig", download_count: 9 },
        { name: "sumi_0.52.1_x64-setup.exe", download_count: 6 },
        { name: "sumi_0.52.1_x64-setup.exe.sig", download_count: 9 },
      ],
    },
    { tag_name: "v0.0.0", published_at: "", draft: true, assets: [] },
  ]);
  assert.deepEqual(summary.totals, {
    mac_installer: 1,
    mac_update: 4,
    windows_installer: 6,
  });
  assert.equal(summary.releases.length, 1);
});

test("requests without the key get nothing, and only aggregates come back", async () => {
  const queries = [];
  const sql = async (query) => {
    queries.push(query);
    if (query.includes("as people,"))
      return [
        {
          people: 3,
          active_today: 1,
          active_7d: 2,
          active_30d: 3,
          new_7d: 1,
          seen_24h: 1,
          tracked_since: "2026-11-01",
        },
      ];
    if (query.includes("row_number()"))
      return [
        {
          n: 7,
          first_seen: "2026-10-28T10:00:00Z",
          last_seen: new Date().toISOString(),
          last_ai: null,
          days_30d: 4,
          hours_7d: 6,
          refreshes_7d: 6,
          refreshes_total: 30,
        },
      ];
    if (query.includes("global:ai"))
      return [
        { ai_today: 5, search_today: 1, ai_yesterday: 7, search_yesterday: 0 },
      ];
    if (query.includes("as minute\n    from tok"))
      return [
        { user_id: "11111111-1111-4111-8111-111111111111", minute: 1000 },
        { user_id: "11111111-1111-4111-8111-111111111111", minute: 1060 },
      ];
    if (query.includes("minute:"))
      return [
        {
          user_key: "11111111-1111-4111-8111-111111111111",
          minute: Math.floor(Date.now() / 60000) - 2,
          calls: 3,
        },
      ];
    return [];
  };
  const fetcher = async () => Response.json([]);
  const handle = createStats({
    env: env({ STATS_KEY: KEY }),
    fetch: fetcher,
    sql,
  });
  const denied = await handle(
    new Request("https://x/buddy-stats", {
      headers: { origin: "https://katenorth7.github.io" },
    }),
  );
  assert.equal(denied.status, 401);
  assert.equal(queries.length, 0);
  const ok = await handle(
    new Request("https://x/buddy-stats", {
      headers: { "x-stats-key": KEY, origin: "https://katenorth7.github.io" },
    }),
  );
  assert.equal(ok.status, 200);
  assert.equal(
    ok.headers.get("access-control-allow-origin"),
    "https://katenorth7.github.io",
  );
  const body = await ok.json();
  assert.equal(body.users.ok, true);
  assert.equal(body.users.data.recent.people_15m, 1);
  assert.equal(body.users.data.app_sessions_30d.median_minutes, 120);
  assert.equal(body.users.data.ai.calls_today, 5);
  assert.deepEqual(Object.keys(body.users.data.installs[0]).sort(), [
    "days_30d",
    "first_seen",
    "hours_7d",
    "label",
    "last_ai",
    "last_seen",
    "refreshes_7d",
    "refreshes_total",
  ]);
  assert.equal(body.users.data.installs[0].label, "#7");
  assert.equal(body.site.ok, false);
  assert.doesNotMatch(JSON.stringify(body), /11111111-1111/);
  const foreign = await handle(
    new Request("https://x/buddy-stats", {
      headers: { "x-stats-key": KEY, origin: "https://evil.example" },
    }),
  );
  assert.equal(foreign.headers.get("access-control-allow-origin"), null);
});
