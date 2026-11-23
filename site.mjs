import { video, analytics } from "./site-config.mjs?v=fork-ga4-1";
import { RELEASE_API, selectDownloads, youtubeEmbed } from "./releases.mjs";
import { installAnalytics, trackEvent } from "./analytics.mjs";

try {
  if (analytics.measurementId) installAnalytics(analytics.measurementId);
} catch {
  // A malformed ID leaves the page without analytics rather than broken.
}

const player = document.querySelector("#player");
const play = document.querySelector("#play-video");
const status = document.querySelector("#download-status");

try {
  const embed = youtubeEmbed(video.youtubeId);
  document.querySelector("#video-poster").src = video.poster;
  document.querySelector("#video-error a").href =
    `https://www.youtube.com/watch?v=${video.youtubeId}`;
  play.hidden = false;
  play.addEventListener(
    "click",
    () => {
      trackEvent("play_video", { video_id: video.youtubeId });
      const iframe = document.createElement("iframe");
      iframe.src = embed;
      iframe.title = "Meet Sumi — Your AI Desktop Companion";
      iframe.allow =
        "accelerometer; autoplay; encrypted-media; gyroscope; picture-in-picture; fullscreen";
      iframe.allowFullscreen = true;
      iframe.referrerPolicy = "strict-origin-when-cross-origin";
      player.replaceChildren(iframe);
    },
    { once: true },
  );
} catch {
  // The ordinary Watch on YouTube link stays usable if a future config is invalid.
  play.hidden = true;
}

// Outbound clicks: downloads (with the resolved version), video link, GitHub, Privacy.
let resolvedVersion = "unknown";
for (const [selector, name, params] of [
  ["#download-windows", "download", { platform: "windows" }],
  ["#download-macos", "download", { platform: "macos" }],
  ["#video-error a", "watch_on_youtube", {}],
  ["#github-link", "open_github", {}],
  ["#privacy-link", "open_privacy", {}],
]) {
  document.querySelector(selector)?.addEventListener("click", () => {
    trackEvent(name, { ...params, app_version: resolvedVersion });
  });
}

// One unauthenticated metadata request to the release repository; no tokens or storage.
// If GitHub is unavailable/rate-limited, both ordinary links still open Releases.
async function loadDownloads() {
  try {
    const response = await fetch(RELEASE_API, {
      headers: { Accept: "application/vnd.github+json" },
      signal: AbortSignal.timeout(8000),
      credentials: "omit",
    });
    if (!response.ok) throw new Error("Release metadata unavailable");
    const downloads = selectDownloads(await response.json());
    document.querySelector("#download-windows").href = downloads.windows;
    document.querySelector("#download-macos").href = downloads.macos;
    document.querySelector("#release-version").textContent = downloads.version;
    resolvedVersion = downloads.version;
  } catch {
    status.hidden = false;
  }
}
loadDownloads();
