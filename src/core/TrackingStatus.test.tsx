import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { TrackingStatus } from "./TrackingStatus";
import { DaySummary } from "./Progress";
import type { Dashboard } from "../types";
import { defaultCompanionView } from "../companion/types";
import { Experience } from "./Experience";

const dashboard: Pick<Dashboard, "status" | "goal_matching" | "goal"> = {
  status: {
    tracking: false,
    ai_enabled: false,
    dnd: false,
    demo: false,
    mock_ai: false,
    anthropic_configured: false,
    web_search_configured: false,
  },
  goal: { id: 1, text: "Build dashboard", created_at: "2026-10-08T00:00:00Z" },
};
const render = (value = dashboard) =>
  renderToStaticMarkup(
    createElement(TrackingStatus, { dashboard: value, onSettings: () => {} }),
  );

describe("goal activity collection status", () => {
  it("keeps the diagnostics panel out of Today", () => {
    const html = renderToStaticMarkup(
      createElement(Experience, {
        page: "focus",
        onDirty: () => {},
        onChanged: async () => {},
        revision: "test",
        onSettings: () => {},
        dashboard: {
          ...dashboard,
          buddy: { companion: defaultCompanionView },
        } as Dashboard,
      }),
    );
    expect(html).not.toContain('aria-label="Activity tracking status"');
    expect(html).not.toContain("Tracking details");
  });
  it("shows a paused collector even when a goal is selected", () => {
    const html = render();
    expect(html).toContain("Activity tracking is paused");
    expect(html).toContain("Set up activity tracking");
    expect(html).not.toContain("tracking is on");
  });
  it("shows permission failures before claiming collection is running", () => {
    const html = render({
      ...dashboard,
      status: {
        ...dashboard.status,
        tracking: true,
        tracking_error: "Allow sumi in Accessibility settings",
      },
    });
    expect(html).toContain("tracking is blocked");
    expect(html).toContain("Accessibility settings");
    expect(html).not.toContain("tracking is on");
  });
  it("does not claim a selected goal is being tracked while AI is uncertain", () => {
    const html = render({
      ...dashboard,
      status: { ...dashboard.status, tracking: true },
      goal_matching: {
        enabled: true,
        goal_id: null,
        confidence: null,
        reason: "No confident match",
      },
    });
    expect(html).toContain("awaiting a goal match");
    expect(html).toContain("No confident match");
    expect(html).not.toContain("tracking is on");
  });
  it("shows active collection with explicit completion controls", () => {
    expect(
      render({
        ...dashboard,
        status: {
          ...dashboard.status,
          tracking: true,
          last_collected_at: new Date().toISOString(),
        },
      }),
    ).toContain("Activity tracking is on · Build dashboard");
    expect(render()).toContain("Mark steps and goals");
  });
  it.each([undefined, "2000-01-01T00:00:00Z", "invalid"])(
    "does not claim collection is current with heartbeat %s",
    (last_collected_at) => {
      const html = render({
        ...dashboard,
        status: { ...dashboard.status, tracking: true, last_collected_at },
      });
      expect(html).toContain("Waiting for activity from your work app");
      expect(html).not.toContain("Activity tracking is on");
      expect(html).not.toContain("Activity tracking is on");
    },
  );
  it("does not interpret missing records as a quiet work day", () => {
    const html = renderToStaticMarkup(
      createElement(DaySummary, {
        day: { day: "2026-10-08", completed: [], goals: [] },
      }),
    );
    expect(html).toContain("No recorded activity or completed goals");
    expect(html).not.toContain("quieter day");
  });
  it("shows unfinished work and the actual AI gate without treating elapsed time as completion", () => {
    const html = renderToStaticMarkup(
      createElement(TrackingStatus, {
        dashboard: {
          ...dashboard,
          status: {
            ...dashboard.status,
            tracking: true,
            last_collected_at: "2026-10-12T09:00:00Z",
          },
          goal_plan: {
            goal_id: 1,
            revision: 1,
            done_when: "Report sent",
            current_step: "done",
            steps: [
              { id: "done", text: "Collect source material", done: true },
              { id: "next", text: "Write the summary", done: false },
            ],
          },
          buddy: {
            companion: {
              ...defaultCompanionView,
              progress_status: "Waiting for a completion receipt.",
            },
          } as Dashboard["buddy"],
        },
        onSettings: () => {},
        compact: true,
        now: Date.parse("2026-10-12T09:00:03Z"),
      }),
    );
    expect(html).toContain("Next unfinished step");
    expect(html).toContain("Write the summary");
    expect(html).not.toContain("Collect source material");
    expect(html).toContain("1 / 2 steps complete");
    expect(html).toContain("Waiting for a completion receipt.");
    expect(html).toContain('dateTime="2026-10-12T09:00:00Z"');
    expect(html).toContain("always need your confirmation");
  });
});
