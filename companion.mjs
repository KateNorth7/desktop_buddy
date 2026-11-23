// The landing page's companion mirrors the app: the same card kinds, titles,
// wording and buttons as InterventionCard, and the same mascot poses as
// Avatar.tsx. Nothing here talks to a server.

// How long sumi stays in the success pose after a confirmed completion (app: 1600 ms),
// and how long the mascot waits alone before the next card appears.
export const SUCCESS_MS = 1600;
export const NEXT_CARD_MS = 5000;
export const FRAME_MS = 500;

const GOAL = "Prepare the autumn release";

export const COMPANION_CARDS = Object.freeze([
  {
    kind: "goal_completion",
    title: "Mark this goal complete?",
    text: `It looks like you finished “${GOAL}”.`,
    primary: { id: "accept", label: "Yes, complete goal", celebrate: true },
    secondary: { id: "ignore", label: "Not yet" },
    pose: "attention",
  },
  {
    kind: "deadline",
    title: "An upcoming deadline",
    text: `“${GOAL}” is due at 10:00.`,
    primary: { id: "on_it", label: "I’m on it" },
    secondary: { id: "reschedule", label: "Reschedule", reschedule: true },
    pose: "idle",
  },
  {
    kind: "unplanned_work",
    title: "Add this work to Today?",
    text: "You spent 40 minutes in Figma on the release mockups.",
    primary: { id: "accept", label: "Add to Today" },
    secondary: { id: "ignore", label: "Skip" },
    pose: "attention",
  },
  {
    kind: "automatic_completion",
    title: "You completed this work",
    text: `Closed “${GOAL}”.`,
    primary: { id: "ignore", label: "Got it" },
    secondary: { id: "accept", label: "Undo" },
    pose: "success",
  },
]);

export const RESCHEDULE = Object.freeze({
  title: "Reschedule your deadline",
  text: "Choose a new date and time.",
  field: "New deadline",
  save: "Save deadline",
  cancel: "Cancel",
});

// --- The mascot, as drawn by the app ---------------------------------------
// An 8 x 8 body on a 12 x 12 cell grid (body at columns and rows 2-9); eyes,
// mouth and margin marks on a quarter-cell grid. Rectangles are
// [x, y, width, height, paint] in quarter cells.

const CELL = 7.5;
const Q = CELL / 4;
export const AVATAR_VIEWBOX = `${-2 * CELL} ${-2 * CELL} ${12 * CELL} ${12 * CELL}`;

export const FILL = Object.freeze({
  body: "#e4fa8e",
  eye: "#1E2A23",
  ink: "#1E2A23",
  mark: "#1f1f1f",
  accent: "#6c5fff",
});

const BODY = [
  "00111100",
  "01111110",
  "11111111",
  "11111111",
  "11111111",
  "11111111",
  "01111110",
  "00111100",
];

function body(ox, oy, rows = BODY) {
  const out = [];
  rows.forEach((row, y) =>
    row.split("").forEach((cell, x) => {
      if (cell === "1") out.push([(x + ox) * 4, (y + oy) * 4, 4, 4, "body"]);
    }),
  );
  return out;
}
const dot = (cx, cy, paint = "mark") => [cx * 4 + 1, cy * 4 + 1, 2, 2, paint];

function eyes(kind, ox, oy) {
  const out = [];
  for (const col of [ox + 2, ox + 5]) {
    const r = oy + 3;
    if (kind === "alert") out.push(dot(col, r, "eye"), dot(col, r + 1, "eye"));
    if (kind === "up") out.push(dot(col, r - 1, "eye"), dot(col, r, "eye"));
  }
  return out;
}
const smile = (ox, oy) => [
  dot(ox + 2.5, oy + 5, "ink"),
  [(ox + 3) * 4 + 2, (oy + 5) * 4 + 3, 4, 2, "ink"],
  dot(ox + 4.5, oy + 5, "ink"),
];
const face = (kind, mouth = null, ox = 2, oy = 2) => [
  ...body(ox, oy),
  ...eyes(kind, ox, oy),
  ...(mouth ? mouth(ox, oy) : []),
];
const trail = (n) => [dot(9, 2), dot(10, 1), dot(11, 0)].slice(0, n);
const question = [
  [9 * 4, 0, 6, 2, "mark"],
  [11 * 4, 2, 2, 2, "mark"],
  [10 * 4, 4, 4, 2, "mark"],
  dot(10, 1.5),
  dot(10, 2.75),
];
const confetti = [
  [dot(0.5, 1, "accent"), dot(11, 2.5), dot(1, 10.5), dot(10.5, 10, "accent")],
  [
    dot(1.5, 0),
    dot(10, 0.5, "accent"),
    dot(0, 9, "accent"),
    dot(11, 9.5),
    dot(5.5, 0, "accent"),
  ],
  [dot(0, 0.5), dot(11, 0), dot(0.5, 11, "accent"), dot(10.5, 11, "accent")],
];

// Moods by frame, as in the app's FRAMES table.
export const FRAMES = Object.freeze({
  working: [face("alert")],
  thinking: [
    [...face("up"), ...trail(1)],
    [...face("up"), ...trail(2)],
    [...face("up"), ...trail(3)],
  ],
  done: [
    [...face("alert", smile), ...confetti[0]],
    [...face("alert", smile, 2, 1), ...confetti[1]],
    [...face("alert", smile), ...confetti[2]],
  ],
  attention: [[...face("alert"), ...question]],
});

// Avatar states map onto moods exactly as the app's moodOf().
export function moodOf(state) {
  switch (state) {
    case "thinking":
      return "thinking";
    case "completed":
    case "success":
    case "fun":
      return "done";
    case "attention":
      return "attention";
    default:
      return "working";
  }
}

// One frame of a state as SVG rect attributes in the app's coordinate system.
export function avatarFrame(state, frame = 0) {
  const frames = FRAMES[moodOf(state)];
  return frames[frame % frames.length].map(([x, y, w, h, paint]) => ({
    x: (x - 8) * Q,
    y: (y - 8) * Q,
    width: w * Q,
    height: h * Q,
    fill: FILL[paint],
    eye: paint === "eye",
  }));
}

export function frameCount(state) {
  return FRAMES[moodOf(state)].length;
}
