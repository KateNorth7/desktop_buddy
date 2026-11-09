// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CoreOnboarding } from "./Onboarding";
import { GoalRow } from "./Goals";
import { defaultUserSettings } from "../types";
import { emptyCore, type CoreGoal } from "./types";
import { InterventionCard } from "../companion/InterventionCard";

const { setup, invoke, platform } = vi.hoisted(() => ({
  setup: vi.fn(),
  invoke: vi.fn(),
  platform: { isMac: false },
}));
vi.mock("../api/tauri", () => ({ desktop: true }));
vi.mock("../api/platform", () => ({
  get isMac() {
    return platform.isMac;
  },
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./api", () => ({ coreApi: { setup } }));
vi.mock("../components/MacPermissions", () => ({
  MacPermissions: () => (
    <div aria-label="macOS permissions">Accessibility setup</div>
  ),
}));
beforeEach(() => {
  platform.isMac = false;
  setup.mockReset().mockResolvedValue(undefined);
  invoke.mockReset().mockResolvedValue(true);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
function openChoices() {
  const changed = vi.fn().mockResolvedValue(undefined);
  const complete = vi.fn();
  render(
    <CoreOnboarding
      initial={defaultUserSettings}
      onChanged={changed}
      onDirty={vi.fn()}
      onComplete={complete}
    />,
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Your name" }), {
    target: { value: "Sam" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByText("WELCOME · 2 OF 3")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(
    screen.getByRole("heading", { name: "How would you like to start?" }),
  ).toBeTruthy();
  return { changed, complete };
}

it("does not treat the preselected tracking default as an unsaved edit", () => {
  const dirty = vi.fn();
  render(
    <CoreOnboarding
      initial={defaultUserSettings}
      onChanged={vi.fn()}
      onDirty={dirty}
    />,
  );
  expect(dirty).toHaveBeenLastCalledWith(false);
});

it.each([false, true])(
  "defaults to tracking and opens native setup only on Continue (macOS=%s)",
  async (mac) => {
    platform.isMac = mac;
    const { changed, complete } = openChoices();
    expect(
      (
        screen.getByRole("radio", {
          name: "With tracking",
        }) as HTMLInputElement
      ).checked,
    ).toBe(true);
    expect(
      screen.getByText("Notice work, time spent, and finished steps."),
    ).toBeTruthy();
    expect(screen.getByLabelText("Tracking setup")).toBeTruthy();
    expect(invoke).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(complete).toHaveBeenCalledTimes(1));
    expect(setup).toHaveBeenCalledWith(
      "Sam",
      [],
      true,
      expect.any(String),
      "",
      defaultUserSettings.profile.avatar,
    );
    expect(invoke.mock.calls).toEqual(
      mac
        ? [
            ["request_accessibility_permission"],
            ["screenshot_permission", { request: true }],
          ]
        : [["request_accessibility_permission"]],
    );
    expect(changed).toHaveBeenCalledTimes(1);
  },
);

it("honors an explicit manual-mode choice without requesting system access", async () => {
  const { complete } = openChoices();
  fireEvent.click(screen.getByRole("radio", { name: "Without tracking" }));
  expect(screen.queryByLabelText("Tracking setup")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  await waitFor(() => expect(complete).toHaveBeenCalledOnce());
  expect(setup.mock.calls[0][2]).toBe(false);
  expect(invoke).not.toHaveBeenCalled();
});

it.each([false, true])(
  "opens platform setup only when With tracking is selected (macOS=%s)",
  async (mac) => {
    platform.isMac = mac;
    const { complete } = openChoices();
    fireEvent.click(screen.getByRole("radio", { name: "Without tracking" }));
    fireEvent.click(screen.getByRole("radio", { name: "With tracking" }));
    expect(setup).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.queryByText("Opening tracking setup…")).toBeNull(),
    );
    if (mac) expect(screen.getByLabelText("macOS permissions")).toBeTruthy();
    expect(screen.queryByText(/Windows needs no extra permission/)).toBeNull();
    expect(screen.queryByText("What is shared?")).toBeNull();
    expect(screen.queryByText("Check again")).toBeNull();
    expect(screen.queryByLabelText(/API key/i)).toBeNull();
    expect(screen.queryByText(/Connect Claude/i)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(complete).toHaveBeenCalledTimes(1));
    expect(setup).toHaveBeenCalledWith(
      "Sam",
      [],
      true,
      expect.any(String),
      "",
      defaultUserSettings.profile.avatar,
    );
    expect(invoke.mock.calls).toEqual(
      mac
        ? [
            ["request_accessibility_permission"],
            ["screenshot_permission", { request: true }],
          ]
        : [["request_accessibility_permission"]],
    );
  },
);

it("switches back to manual mode without enabling tracking or reopening system setup", async () => {
  const { complete } = openChoices();
  fireEvent.click(screen.getByRole("radio", { name: "Without tracking" }));
  fireEvent.click(screen.getByRole("radio", { name: "With tracking" }));
  await waitFor(() =>
    expect(screen.queryByText("Opening tracking setup…")).toBeNull(),
  );
  fireEvent.click(screen.getByRole("radio", { name: "Without tracking" }));
  expect(screen.queryByLabelText("Tracking setup")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  await waitFor(() => expect(complete).toHaveBeenCalledTimes(1));
  expect(setup.mock.calls[0][2]).toBe(false);
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("keeps the card recoverable if system setup cannot open", async () => {
  invoke.mockRejectedValueOnce("System settings unavailable");
  openChoices();
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(
      "System settings unavailable",
    ),
  );
  expect(setup).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("radio", { name: "Without tracking" }));
  expect(screen.queryByRole("alert")).toBeNull();
});

it("opens the setup goal's steps and shows the installation checked with its spent time", () => {
  const snapshot = emptyCore();
  const goal: CoreGoal = {
    id: 7,
    title: "Install and start using Sumi",
    status: "open",
    area_id: 1,
    focused_seconds: 300,
    relevant_seconds: 300,
    plan: {
      goal_id: 7,
      revision: 2,
      done_when: "",
      current_step: null,
      steps: [
        { id: "step-0", text: "Find and install the Sumi app", done: true },
        { id: "step-1", text: "Create a new goal", done: false },
        {
          id: "step-2",
          text: "Check out My progress in the menu",
          done: false,
        },
      ],
    },
  };
  snapshot.onboarding_goal_id = goal.id;
  snapshot.goals = [goal];
  snapshot.today = [goal.id];
  render(
    <GoalRow goal={goal} snapshot={snapshot} busy={false} run={vi.fn()} />,
  );
  const install = screen.getByRole("checkbox", {
    name: "Find and install the Sumi app",
  }) as HTMLInputElement;
  expect(install.checked).toBe(true);
  expect(screen.getByText("5 mins spent")).toBeTruthy();
  expect(
    (
      screen.getByRole("checkbox", {
        name: "Create a new goal",
      }) as HTMLInputElement
    ).checked,
  ).toBe(false);
  expect(
    screen
      .getByRole("button", { name: /Hide steps/ })
      .getAttribute("aria-expanded"),
  ).toBe("true");
});

it.each([
  [
    "step-0",
    "Welcome to Sumi!",
    "You’ve installed the app. Let’s cross off your first step.",
    "Cross it off",
  ],
  [
    "step-1",
    "Your first goal is ready",
    "Cross off this step, then open My progress to see your time and finished work.",
    "Cross off & view progress",
  ],
  [
    "step-2",
    "You’re all set!",
    "Your first 5 minutes are in My progress. Cross off this last step to finish setup.",
    "Finish setup",
  ],
])(
  "renders option A for %s with one clear next action",
  (step, title, body, primary) => {
    const respond = vi.fn();
    render(
      <InterventionCard
        busy={false}
        respond={respond}
        prompt={{
          id: `sumi-setup:7:${step}`,
          kind: "onboarding_completion",
          text: body,
          confidence: 1,
          goal_id: 7,
          step_id: step,
          plan_revision: 2,
          expires_at: Date.now() / 1000 + 300,
        }}
      />,
    );
    expect(screen.getByRole("heading", { name: title })).toBeTruthy();
    expect(screen.getByText(body)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Chat" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Skip today" })).toBeNull();
    if (step === "step-0")
      expect(
        screen.getByText("Next: create a goal. I can suggest ideas."),
      ).toBeTruthy();
    if (step === "step-2") {
      expect(screen.queryByRole("button", { name: "Later" })).toBeNull();
      expect(
        screen.getByRole("button", { name: "Dismiss setup message" }),
      ).toBeTruthy();
    } else expect(screen.getByRole("button", { name: "Later" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: primary }));
    expect(respond).toHaveBeenCalledWith("accept");
  },
);
