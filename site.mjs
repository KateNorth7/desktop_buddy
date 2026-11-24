import { video, analytics } from "./site-config.mjs?v=landing-6";
import { RELEASE_API, selectDownloads, youtubeEmbed } from "./releases.mjs?v=landing-6";
import { installAnalytics, trackEvent } from "./analytics.mjs?v=landing-6";
import {
  COMPANION_CARDS,
  RESCHEDULE,
  SUCCESS_MS,
  NEXT_CARD_MS,
  FRAME_MS,
  AVATAR_VIEWBOX,
  avatarFrame,
  frameCount,
} from "./companion.mjs?v=landing-6";

import { readConsent, writeConsent } from "./consent.mjs?v=landing-6";

// Analytics only after consent. Declining keeps the page free of any Google request.
const banner = document.querySelector("#consent");
function startAnalytics() {
  try {
    if (analytics.measurementId) installAnalytics(analytics.measurementId);
  } catch {
    // A malformed ID leaves the page without analytics rather than broken.
  }
}
function decide(value) {
  writeConsent(localStorage, value);
  banner.hidden = true;
  if (value === "accepted") startAnalytics();
}
if (analytics.measurementId) {
  const choice = readConsent(localStorage);
  if (choice === "accepted") startAnalytics();
  else if (choice === null) {
    banner.hidden = false;
    document
      .querySelector("#consent-accept")
      .addEventListener("click", () => decide("accepted"));
    document
      .querySelector("#consent-decline")
      .addEventListener("click", () => decide("declined"));
  }
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

// --- The companion ---------------------------------------------------------
// The mascot is the app's own pixel avatar; cards use the app's kinds, wording
// and buttons. A card waits for a click. Confirming a completion puts sumi in
// the success pose for SUCCESS_MS; the next card arrives after NEXT_CARD_MS.

const SVG = "http://www.w3.org/2000/svg";
const slot = document.querySelector("#bubble-slot");
const mascotHost = document.querySelector("#mascot");
const reducedMotion = window.matchMedia?.(
  "(prefers-reduced-motion: reduce)",
).matches;

let avatarState = "idle";
let frameIndex = 0;
let frameTimer = null;

function drawAvatar() {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", AVATAR_VIEWBOX);
  svg.setAttribute("shape-rendering", "crispEdges");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", `sumi companion, ${avatarState}`);
  svg.setAttribute("class", `avatar avatar-${avatarState}`);
  const bodyGroup = document.createElementNS(SVG, "g");
  const eyeGroup = document.createElementNS(SVG, "g");
  eyeGroup.setAttribute("class", "avatar-eyes");
  for (const cell of avatarFrame(avatarState, frameIndex)) {
    const rect = document.createElementNS(SVG, "rect");
    rect.setAttribute("x", cell.x);
    rect.setAttribute("y", cell.y);
    rect.setAttribute("width", cell.width);
    rect.setAttribute("height", cell.height);
    rect.setAttribute("fill", cell.fill);
    (cell.eye ? eyeGroup : bodyGroup).append(rect);
  }
  bodyGroup.append(eyeGroup);
  svg.append(bodyGroup);
  mascotHost.replaceChildren(svg);
}

function setAvatar(state) {
  avatarState = state;
  frameIndex = 0;
  clearInterval(frameTimer);
  drawAvatar();
  const count = frameCount(state);
  if (count > 1 && !reducedMotion) {
    frameTimer = setInterval(() => {
      frameIndex = (frameIndex + 1) % count;
      drawAvatar();
    }, FRAME_MS);
  }
}

let cardIndex = 0;
let nextTimer = null;
let successTimer = null;

function button(label, className, onClick) {
  const el = document.createElement("button");
  el.type = "button";
  el.textContent = label;
  if (className) el.className = className;
  el.addEventListener("click", onClick);
  return el;
}

function cardShell(title, text) {
  const section = document.createElement("section");
  section.className = "suggestion-card companion-prompt";
  section.setAttribute("aria-label", title);
  const h2 = document.createElement("h2");
  h2.textContent = title;
  const p = document.createElement("p");
  p.textContent = text;
  section.append(h2, p);
  return section;
}

function showCard(card) {
  const section = cardShell(card.title, card.text);
  const actions = document.createElement("div");
  actions.className = "companion-prompt-actions";
  actions.append(
    button(card.primary.label, "", () => respond(card, card.primary)),
    button(card.secondary.label, "secondary", () =>
      card.secondary.reschedule
        ? showReschedule(card)
        : respond(card, card.secondary),
    ),
  );
  section.append(actions);
  slot.replaceChildren(section);
  slot.hidden = false;
  setAvatar(card.pose);
}

function showReschedule(card) {
  const section = cardShell(RESCHEDULE.title, RESCHEDULE.text);
  const label = document.createElement("label");
  label.className = "setting-row";
  const span = document.createElement("span");
  span.textContent = RESCHEDULE.field;
  const input = document.createElement("input");
  input.type = "datetime-local";
  const tomorrow = new Date(Date.now() + 86400000);
  tomorrow.setHours(10, 0, 0, 0);
  const pad = (n) => String(n).padStart(2, "0");
  input.value = `${tomorrow.getFullYear()}-${pad(tomorrow.getMonth() + 1)}-${pad(tomorrow.getDate())}T10:00`;
  label.append(span, input);
  const actions = document.createElement("div");
  actions.className = "companion-prompt-actions";
  actions.append(
    button(RESCHEDULE.save, "", () =>
      respond(card, { id: "reschedule", label: RESCHEDULE.save }),
    ),
    button(RESCHEDULE.cancel, "secondary", () => showCard(card)),
  );
  section.append(label, actions);
  slot.replaceChildren(section);
}

function respond(card, choice) {
  clearTimeout(nextTimer);
  clearTimeout(successTimer);
  trackEvent("companion_reply", { card: card.kind, choice: choice.id });
  slot.replaceChildren();
  slot.hidden = true;
  if (choice.celebrate) {
    setAvatar("success");
    successTimer = setTimeout(() => setAvatar("idle"), SUCCESS_MS);
  } else {
    setAvatar("idle");
  }
  nextTimer = setTimeout(() => {
    cardIndex = (cardIndex + 1) % COMPANION_CARDS.length;
    showCard(COMPANION_CARDS[cardIndex]);
  }, NEXT_CARD_MS);
}

if (slot && mascotHost) showCard(COMPANION_CARDS[cardIndex]);

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
