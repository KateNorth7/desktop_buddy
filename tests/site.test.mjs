import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { selectDownloads, youtubeEmbed, LATEST_RELEASE, REPOSITORY } from "../releases.mjs";
import { gtagScriptUrl } from "../analytics.mjs";
import { analytics } from "../site-config.mjs";
import { COMPANION_CARDS, REPLY_MS } from "../companion.mjs";
import { existsSync } from "node:fs";
const release = {
  tag_name: "v0.49.4",
  draft: false,
  prerelease: false,
  assets: ["sumi_0.49.4_aarch64.dmg", "sumi_0.49.4_x64-setup.exe"].map(
    (name) => ({
      name,
      state: "uploaded",
      size: 1,
      browser_download_url: `https://github.com/KateNorth7/desktop_buddy/releases/download/v0.49.4/${name}`,
    }),
  ),
};
test("select exact current supported installer pair", () => {
  const d = selectDownloads(release);
  assert.match(d.macos, /aarch64.dmg$/);
  assert.match(d.windows, /x64-setup.exe$/);
  assert.equal(d.version, "v0.49.4");
});
test("future releases resolve without a site deploy", () => {
  const next = JSON.parse(
    JSON.stringify(release).replaceAll("0.49.4", "0.50.0"),
  );
  assert.match(selectDownloads(next).windows, /v0.50.0/);
});
test("reject incomplete or unuploaded releases", () => {
  assert.throws(() =>
    selectDownloads({ ...release, assets: release.assets.slice(1) }),
  );
  assert.throws(() =>
    selectDownloads({
      ...release,
      assets: release.assets.map((a) => ({ ...a, state: "new" })),
    }),
  );
});
test("reject substituted external installer destinations", () => {
  const bad = structuredClone(release);
  bad.assets[0].browser_download_url = "https://example.com/app.dmg";
  assert.throws(() => selectDownloads(bad));
});
test("reject draft, prerelease and malformed tags", () => {
  for (const patch of [
    { draft: true },
    { prerelease: true },
    { tag_name: "v1/../../x" },
    { tag_name: "v1.0.0-rc.1" },
  ])
    assert.throws(() => selectDownloads({ ...release, ...patch }));
});
test("YouTube accepts IDs only, creates privacy-enhanced embed", () => {
  assert.equal(
    youtubeEmbed("4Vu0HkIB1gY"),
    "https://www.youtube-nocookie.com/embed/4Vu0HkIB1gY?rel=0&playsinline=1&autoplay=1",
  );
  for (const id of ["", "https://youtube.com/watch?v=x", 'x" onload=bad'])
    assert.throws(() => youtubeEmbed(id));
});
test("HTML keeps functional release-page links without JavaScript", () => {
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  assert.equal(html.split(`href="${LATEST_RELEASE}"`).length - 1, 2);
  assert.match(html, /docs\/privacy.html/);
  assert.match(html, /https:\/\/www\.youtube\.com\/watch\?v=4Vu0HkIB1gY/);
  assert.doesNotMatch(html, /<video\b|\.mp4|demo-poster\.jpg/);
});
test("releases and links come from the fork that publishes installers", () => {
  assert.equal(REPOSITORY, "KateNorth7/desktop_buddy");
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  assert.doesNotMatch(html, /github\.com\/ArtmarketVM/);
  assert.match(html, /id="github-link"/);
  assert.match(html, /id="privacy-link"/);
});
test("companion cards are the app's own, with a reply for every choice", () => {
  assert.equal(COMPANION_CARDS.length, 3);
  assert.deepEqual(
    COMPANION_CARDS.map((c) => c.id),
    ["completion", "deadline", "new_work"],
  );
  for (const card of COMPANION_CARDS) {
    assert.equal(card.choices.length, 2);
    for (const choice of card.choices) assert.ok(choice.reply.length > 0);
  }
  assert.equal(REPLY_MS, 5000);
  assert.ok(existsSync(new URL("../assets/sumi-mascot.png", import.meta.url)));
  assert.ok(
    existsSync(
      new URL(
        "../assets/fonts/instrument-sans-latin-600-normal.woff2",
        import.meta.url,
      ),
    ),
  );
});
test("GA4 loads only for a well-formed measurement ID", () => {
  assert.equal(
    gtagScriptUrl("G-ABC123XYZ"),
    "https://www.googletagmanager.com/gtag/js?id=G-ABC123XYZ",
  );
  for (const id of ["", "UA-1234-1", "G-abc", 'G-1" onload=bad', "GTM-ABCD"])
    assert.throws(() => gtagScriptUrl(id));
  if (analytics.measurementId) gtagScriptUrl(analytics.measurementId);
});
