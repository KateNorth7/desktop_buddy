// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Progress, trackingLine } from "./Progress";
import { emptyCore, type CoreSnapshot, type ProgressPeriod } from "./types";

afterEach(cleanup);
function fixture(): CoreSnapshot {
  const snapshot = emptyCore();
  snapshot.date = "2026-10-24";
  snapshot.summary = {
    day: snapshot.date,
    completed: [],
    goals: [
      {
        goal_id: 7,
        title: "Write the pitch",
        area: "",
        seconds: 120,
        relevant_seconds: 120,
      },
      {
        goal_id: 3,
        title: "Reply to customers",
        area: "",
        seconds: 60,
        relevant_seconds: 60,
      },
    ],
  };
  snapshot.week = [snapshot.summary];
  snapshot.goals = snapshot.summary.goals.map((goal) => ({
    id: goal.goal_id,
    title: goal.title,
    status: "open",
    area_id: 1,
    focused_seconds: 0,
    plan: {
      goal_id: goal.goal_id,
      revision: 1,
      done_when: "Done",
      current_step: null,
      steps: [{ id: "draft", text: "Draft a reply", done: false }],
    },
  }));
  snapshot.hourly = [
    { hour: 9, goal_id: 7, seconds: 120 },
    { hour: 9, goal_id: 3, seconds: 60 },
  ];
  return snapshot;
}
function view(snapshot: CoreSnapshot, period: ProgressPeriod = "week") {
  return (
    <Progress
      snapshot={snapshot}
      period={period}
      anchor={null}
      busy={false}
      onWeek={vi.fn()}
      onPeriod={vi.fn()}
    />
  );
}
function segments() {
  return within(screen.getByLabelText("Focused time")).getAllByRole("button");
}

it("shows details for one segment and opens that goal, including by keyboard focus", () => {
  render(view(fixture()));
  const [pitch, emails] = segments();
  expect(emails.getAttribute("aria-label")).toBe(
    "Reply to customers · Sat · 1m",
  );
  fireEvent.mouseEnter(emails);
  expect(screen.getByRole("tooltip").textContent).toBe(
    "Reply to customersSat · 1m",
  );
  expect(screen.getByRole("tooltip").textContent).not.toContain("pitch");
  fireEvent.mouseLeave(emails);
  expect(screen.queryByRole("tooltip")).toBeNull();
  fireEvent.focus(pitch);
  expect(screen.getByRole("tooltip").textContent).toContain("Write the pitch");
  expect(pitch.getAttribute("aria-describedby")).toBe(
    screen.getByRole("tooltip").id,
  );
  fireEvent.blur(pitch);
  expect(screen.queryByRole("tooltip")).toBeNull();
  fireEvent.click(emails);
  expect(
    screen.getByRole("heading", { name: "Reply to customers" }),
  ).toBeTruthy();
  expect(screen.getByText("Draft a reply")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Progress" }));
  expect(segments()).toHaveLength(2);
});

it("ranks the list and every date stack by period total, and keeps shades during drilldown", () => {
  const snapshot = fixture();
  snapshot.week = [
    snapshot.summary,
    {
      day: "2026-10-23",
      completed: [],
      goals: [
        {
          goal_id: 3,
          title: "Reply to customers",
          area: "",
          seconds: 50,
          relevant_seconds: 50,
        },
        {
          goal_id: 7,
          title: "Write the pitch",
          area: "",
          seconds: 30,
          relevant_seconds: 30,
        },
      ],
    },
  ];
  const { container } = render(view(snapshot));
  const buttons = segments();
  expect(
    buttons.map((button) => button.getAttribute("aria-label")!.split(" · ")[0]),
  ).toEqual([
    "Write the pitch",
    "Reply to customers",
    "Write the pitch",
    "Reply to customers",
  ]);
  const shades = buttons.map((button) =>
    button.style.getPropertyValue("--bar-color"),
  );
  expect(shades[0]).toBe(shades[2]);
  expect(shades[1]).toBe(shades[3]);
  expect(shades[0]).not.toBe(shades[1]);
  expect(shades[0]).toContain("#6c5fff 100.000%");
  expect(
    within(container.querySelector(".progress-list")!)
      .getAllByRole("button")
      .map((button) => button.textContent),
  ).toEqual([
    expect.stringContaining("Write the pitch"),
    expect.stringContaining("Reply to customers"),
  ]);
  fireEvent.click(buttons[1]);
  expect(
    segments().every(
      (button) => button.style.getPropertyValue("--bar-color") === shades[1],
    ),
  ).toBe(true);
});

it("recalculates violet ranks when the selected period's time totals change", () => {
  const snapshot = fixture();
  const { rerender } = render(view(snapshot));
  const darkest = segments()[0].style.getPropertyValue("--bar-color");
  expect(segments()[0].getAttribute("aria-label")).toContain("Write the pitch");
  const next = structuredClone(snapshot);
  next.summary.goals[1].relevant_seconds = 900;
  next.week = [next.summary];
  rerender(view(next));
  expect(segments()[0].getAttribute("aria-label")).toContain(
    "Reply to customers",
  );
  expect(segments()[0].style.getPropertyValue("--bar-color")).toBe(darkest);
});

it("gives equal time equal shades and breaks sorting ties by goal ID", () => {
  const snapshot = fixture();
  snapshot.summary.goals.forEach((goal) => {
    goal.relevant_seconds = 60;
  });
  snapshot.week = [snapshot.summary];
  render(view(snapshot));
  expect(segments()[0].getAttribute("aria-label")).toContain(
    "Reply to customers",
  );
  expect(segments()[0].style.getPropertyValue("--bar-color")).toBe(
    segments()[1].style.getPropertyValue("--bar-color"),
  );
});

it("opens historical goals even when they are absent from today's goal list", () => {
  const snapshot = fixture();
  snapshot.goals = [];
  render(view(snapshot));
  fireEvent.click(segments()[0]);
  expect(screen.getByRole("heading", { name: "Write the pitch" })).toBeTruthy();
  expect(screen.getByText("Other work on this goal")).toBeTruthy();
});

it("does not render daily summary, pending review or work journal controls", () => {
  render(view(fixture(), "day"));
  expect(
    screen.queryByText(/daily summary|pending review|work journal/i),
  ).toBeNull();
});

it("keeps more than six goals distinguishable and a crowded stack inside the plot", () => {
  const snapshot = fixture();
  snapshot.summary.goals = Array.from({ length: 10 }, (_, index) => ({
    goal_id: index + 1,
    title: `Goal ${index + 1}`,
    area: "",
    seconds: index + 1,
    relevant_seconds: index + 1,
  }));
  snapshot.week = [snapshot.summary];
  render(view(snapshot));
  const buttons = segments();
  expect(buttons).toHaveLength(10);
  expect(
    new Set(
      buttons.map((button) => button.style.getPropertyValue("--bar-color")),
    ).size,
  ).toBe(10);
  const gaps = parseFloat(buttons[0].parentElement!.style.gap) * 9;
  expect(
    buttons.reduce(
      (sum, button) => sum + parseFloat(button.style.height),
      gaps,
    ),
  ).toBeLessThanOrEqual(110.01);
});

it("explains recorded time that has not reached a goal and why", () => {
  const snapshot = emptyCore();
  snapshot.date = "2026-10-24";
  snapshot.summary = {
    day: snapshot.date,
    completed: [],
    goals: [],
    tracking: {
      recorded_seconds: 3600,
      attributed_seconds: 600,
      waiting_seconds: 3000,
      issue: "skipped_lane_busy",
    },
  };
  snapshot.week = [snapshot.summary];
  render(view(snapshot, "day"));
  expect(screen.getByLabelText("Tracking status").textContent).toBe(
    "Recorded 1h 0m · 10m matched to goals · 50m waiting for a goal. Goal matching often waited for other AI checks.",
  );
});

it("hides the tracking line when nothing was recorded or an issue code is unknown", () => {
  expect(trackingLine([])).toBeNull();
  expect(
    trackingLine([
      {
        day: "2026-10-24",
        completed: [],
        goals: [],
        tracking: {
          recorded_seconds: 120,
          attributed_seconds: 120,
          waiting_seconds: 0,
          issue: "skipped_lane_busy",
        },
      },
      {
        day: "2026-10-25",
        completed: [],
        goals: [],
        tracking: {
          recorded_seconds: 60,
          attributed_seconds: 0,
          waiting_seconds: 60,
          issue: "unknown_code",
        },
      },
    ]),
  ).toBe("Recorded 3m · 2m matched to goals · 1m waiting for a goal");
});
