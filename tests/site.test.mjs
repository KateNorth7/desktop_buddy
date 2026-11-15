import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { selectDownloads, youtubeEmbed, LATEST_RELEASE } from "../releases.mjs";
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
