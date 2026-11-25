import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  cohortTable,
  formatMinutes,
  formatNumber,
  funnel,
  niceScale,
} from "../stats/model.mjs";

test("the stats page stays out of search engines and loads no third-party code", () => {
  const html = readFileSync(new URL("../stats/index.html", import.meta.url), "utf8");
  assert.match(html, /<meta name="robots" content="noindex, nofollow" \/>/);
  assert.doesNotMatch(html, /googletagmanager|https?:\/\/(?!katenorth7)[^"]+\.js/);
  const script = readFileSync(new URL("../stats/stats.mjs", import.meta.url), "utf8");
  assert.doesNotMatch(script, /innerHTML/);
});

test("durations and numbers read naturally", () => {
  assert.equal(formatMinutes(12), "12 min");
  assert.equal(formatMinutes(60), "1 h");
  assert.equal(formatMinutes(90), "1.5 h");
  assert.equal(formatMinutes(3516), "59 h");
  assert.equal(formatMinutes(null), "–");
  assert.equal(formatNumber(1234), "1,234");
  assert.equal(formatNumber(null), "–");
});

test("a missing source shows as unknown, never as zero", () => {
  const steps = funnel({
    users: { ok: true, data: { people: 19, active_7d: 3 } },
    downloads: { ok: true, data: { totals: { mac_installer: 7, windows_installer: 13, mac_update: 14 } } },
    site: { ok: true, data: { configured: false } },
  });
  assert.deepEqual(steps.map((s) => s.value), [null, null, 20, 19, 3]);
  assert.equal(steps[2].share, 1);
});

test("retention rows keep gaps and scales stay round", () => {
  const rows = cohortTable([
    { cohort: "2026-10-26", week: 0, people: 18 },
    { cohort: "2026-10-26", week: 2, people: 4 },
    { cohort: "2026-11-02", week: 0, people: 1 },
  ]);
  assert.deepEqual(rows[0], { cohort: "2026-10-26", size: 18, weeks: [undefined, 4] });
  assert.deepEqual(niceScale(13), { max: 15, ticks: [0, 5, 10, 15] });
});

test("install status follows session refreshes and AI calls", async () => {
  const { installStatus, relativeTime } = await import("../stats/model.mjs");
  const now = Date.parse("2026-11-25T12:00:00Z");
  assert.equal(installStatus({ last_seen: "2026-11-25T11:00:00Z", last_ai: null }, now).kind, "open");
  assert.equal(installStatus({ last_seen: "2026-11-25T08:00:00Z", last_ai: "2026-11-25T11:50:00Z" }, now).kind, "open");
  assert.equal(installStatus({ last_seen: "2026-11-25T08:00:00Z", last_ai: null }, now).kind, "today");
  assert.equal(installStatus({ last_seen: "2026-11-21T08:00:00Z", last_ai: null }, now).kind, "week");
  assert.equal(installStatus({ last_seen: "2026-11-01T08:00:00Z", last_ai: null }, now).kind, "quiet");
  assert.equal(relativeTime("2026-11-25T11:30:00Z", now), "30 min ago");
  assert.equal(relativeTime("2026-11-24T10:00:00Z", now), "yesterday");
});

test("clicked links read as short labels", async () => {
  const { shortUrl } = await import("../stats/model.mjs");
  assert.equal(shortUrl("https://www.youtube.com/watch?v=4Vu0HkIB1gY"), "YouTube video");
  assert.equal(
    shortUrl("https://github.com/KateNorth7/desktop_buddy/releases/download/v0.52.1/sumi_0.52.1_x64-setup.exe"),
    "GitHub · sumi_0.52.1_x64-setup.exe",
  );
  assert.equal(shortUrl(""), "(not set)");
});

test("tagged links and referrers stay readable and safe", async () => {
  const { taggedLink, referrerLabel } = await import("../stats/model.mjs");
  assert.equal(
    taggedLink({ source: "Telegram Chat", medium: "messenger", campaign: "Testers Nov" }),
    "https://katenorth7.github.io/desktop_buddy/?utm_source=telegram-chat&utm_medium=messenger&utm_campaign=testers-nov",
  );
  assert.equal(taggedLink({}), "https://katenorth7.github.io/desktop_buddy/");
  assert.equal(referrerLabel("https://www.facebook.com/"), "facebook.com");
  assert.equal(referrerLabel("https://news.ycombinator.com/item?id=1"), "news.ycombinator.com/item");
});
