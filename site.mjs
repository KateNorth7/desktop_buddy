import { video, analytics } from "./site-config.mjs?v=landing-2";
import { RELEASE_API, selectDownloads, youtubeEmbed } from "./releases.mjs";
import { installAnalytics, trackEvent } from "./analytics.mjs";
import { COMPANION_CARDS, REPLY_MS } from "./companion.mjs";

try {
  if (analytics.measurementId) installAnalytics(analytics.measurementId);
} catch {
  // A malformed ID leaves the page without analytics rather than broken.
}

const player = document.querySelector("#player");
const play = document.querySelector("#play-video");
const status = document.querySelector("#download-status");
const videoError = document.querySelector("#video-error");

try {
  const embed = youtubeEmbed(video.youtubeId);
  document.querySelector("#video-poster").src = video.poster;
  videoError.querySelector("a").href =
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
  // The ordinary Watch on YouTube link appears if a future config is invalid.
  play.hidden = true;
  videoError.hidden = false;
}

// Outbound clicks: downloads (with the resolved version), GitHub, Privacy.
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

// The companion: one card at a time, waiting for a click; the reply stays
// for REPLY_MS, then the next card appears. Nothing is sent anywhere.
const slot = document.querySelector("#bubble-slot");
const mascot = document.querySelector("#mascot");
let cardIndex = 0;
let replyTimer = null;

function renderCard(card) {
  const bubble = document.createElement("div");
  bubble.className = "bubble";
  const text = document.createElement("span");
  text.append(card.before);
  if (card.goal) {
    const goal = document.createElement("strong");
    goal.textContent = card.goal;
    text.append(goal);
  }
  text.append(card.after);
  const actions = document.createElement("span");
  actions.className = "bubble-actions";
  for (const [i, choice] of card.choices.entries()) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = choice.label;
    if (i === 0) button.className = "primary";
    button.addEventListener("click", () => answer(card, choice));
    actions.append(button);
  }
  bubble.append(text, actions);
  slot.replaceChildren(bubble);
}

function renderReply(choice) {
  const bubble = document.createElement("div");
  bubble.className = "bubble";
  const text = document.createElement("span");
  text.textContent = choice.reply;
  bubble.append(text);
  if (choice.undo) {
    const undo = document.createElement("button");
    undo.type = "button";
    undo.className = "bubble-undo";
    undo.textContent = "Undo";
    bubble.append(undo);
  }
  slot.replaceChildren(bubble);
}

function answer(card, choice) {
  clearTimeout(replyTimer);
  trackEvent("companion_reply", { card: card.id, choice: choice.id });
  mascot.classList.remove("react");
  void mascot.offsetWidth;
  mascot.classList.add("react");
  renderReply(choice);
  replyTimer = setTimeout(() => {
    mascot.classList.remove("react");
    cardIndex = (cardIndex + 1) % COMPANION_CARDS.length;
    renderCard(COMPANION_CARDS[cardIndex]);
  }, REPLY_MS);
}

if (slot && mascot) renderCard(COMPANION_CARDS[cardIndex]);

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
