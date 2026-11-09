// @vitest-environment jsdom
import { queryClient } from "../data/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PersonalAiEnginePanel } from "./PersonalAiEngine";
import { engineApi, type PersonalAiEngine } from "./engineApi";
vi.mock("../api/tauri", () => ({ desktop: true }));
vi.mock("./engineApi", () => ({
  engineApi: { status: vi.fn(), health: vi.fn(), actions: vi.fn() },
}));
const fixture: PersonalAiEngine = {
  provider: "Claude (Anthropic)",
  configured_model: "configured-fixture",
  active_goal: { id: 1, label: "Hackathon" },
  active_step: null,
  work_state: "waiting",
  retrieval: { found: 4, passed: 2, context_bytes: 120 },
  last_decision: "stay_silent",
  last_skill: "park_context",
  last_inference: {
    operation: "buddy_chat",
    status: "success",
    latency_ms: 24,
    provider_reported_model: null,
  },
  tavily_status: "key_configured_not_search_tested",
  runtime: {
    status: "suppressed",
    pending_events: 0,
    diagnostics: {
      ai_calls: 1,
      suppressed: 9,
      cancelled: 0,
      verified: 0,
      suppression_reason: "privacy_or_dnd",
    },
  },
  health: {
    status: "not_checked",
    latency_ms: null,
    provider_reported_model: null,
    error: null,
    checked_at: null,
  },
  skills: [
    {
      group: "Goals",
      skills: [{ name: "create_goal", permission: "reversible" }],
      note: "",
    },
  ],
  demo_mode: false,
  ai_mock: false,
};
beforeEach(() => {
  queryClient.invalidate(undefined, true);
  vi.resetAllMocks();
  vi.mocked(engineApi.status).mockResolvedValue(fixture);
});
afterEach(cleanup);
it("renders the local event journal and runs isolated actions only on request", async () => {
  vi.mocked(engineApi.status).mockResolvedValue({
    ...fixture,
    events: [
      {
        at: "2026-10-14T10:00:00Z",
        stage: "decision",
        operation: "stay_silent",
        status: "verified",
        latency_ms: null,
        goal_id: 1,
      },
    ],
  });
  vi.mocked(engineApi.actions).mockResolvedValue([
    { name: "rename_goal", status: "passed", latency_ms: 20 },
  ]);
  const user = await open();
  expect(screen.getByText(/decision · verified · goal #1/)).toBeTruthy();
  expect(engineApi.actions).not.toHaveBeenCalled();
  await user.click(
    screen.getByRole("button", { name: "Run isolated agent check" }),
  );
  await screen.findByText("rename goal · passed · 20 ms");
  expect(engineApi.actions).toHaveBeenCalledTimes(1);
  expect(engineApi.health).not.toHaveBeenCalled();
});
async function open() {
  render(<PersonalAiEnginePanel />);
  const details = screen.getByText("Personal AI Engine").parentElement!;
  details.setAttribute("open", "");
  fireEvent(details, new Event("toggle"));
  await screen.findByText("configured-fixture");
  return userEvent.setup();
}
it("stays collapsed without requests and opening reads local metadata only", async () => {
  render(<PersonalAiEnginePanel />);
  expect(engineApi.status).not.toHaveBeenCalled();
  expect(engineApi.health).not.toHaveBeenCalled();
  const details = screen.getByText("Personal AI Engine").parentElement!;
  details.setAttribute("open", "");
  fireEvent(details, new Event("toggle"));
  await screen.findByText("configured-fixture");
  expect(engineApi.status).toHaveBeenCalledTimes(1);
  expect(engineApi.health).not.toHaveBeenCalled();
  expect(screen.getByText("Configured model")).toBeTruthy();
  expect(screen.queryByText("Actual model")).toBeNull();
  expect(
    screen.getByText("2 passed / 4 found · 120 context bytes"),
  ).toBeTruthy();
  expect(screen.getByText("key configured not search tested")).toBeTruthy();
});
it("runs one manually requested real inference and distinguishes provider-reported model", async () => {
  vi.mocked(engineApi.health).mockResolvedValue({
    ...fixture.health,
    status: "success",
    latency_ms: 31,
    provider_reported_model: "reported-fixture",
  });
  const user = await open();
  await user.click(
    screen.getByRole("button", { name: "Run inference health check" }),
  );
  await screen.findByText("success · 31 ms");
  expect(engineApi.health).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Provider-reported model")).toBeTruthy();
  expect(screen.getByText("configured-fixture")).toBeTruthy();
  expect(screen.getByText("reported-fixture")).toBeTruthy();
});
it("shows sanitized timeout rather than claiming a configured key passed inference", async () => {
  vi.mocked(engineApi.health).mockResolvedValue({
    ...fixture.health,
    status: "timeout",
    latency_ms: 10000,
    error: "Claude inference timed out after the bounded wait.",
  });
  const user = await open();
  await user.click(
    screen.getByRole("button", { name: "Run inference health check" }),
  );
  await screen.findByText("timeout · 10000 ms");
  expect(screen.getByRole("alert").textContent).toContain("timed out");
  expect(screen.queryByText("Connected")).toBeNull();
});
it("handles local status errors without rendering raw IPC content", async () => {
  vi.mocked(engineApi.status).mockRejectedValue(new Error("secret-canary"));
  render(<PersonalAiEnginePanel />);
  const details = screen.getByText("Personal AI Engine").parentElement!;
  details.setAttribute("open", "");
  fireEvent(details, new Event("toggle"));
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain("Couldn't refresh this information"),
  );
  expect(document.body.textContent).not.toContain("secret-canary");
  expect(engineApi.health).not.toHaveBeenCalled();
});
it("does not infer progress when memory or focus is absent", async () => {
  vi.mocked(engineApi.status).mockResolvedValue({
    ...fixture,
    active_goal: null,
    retrieval: null,
    last_inference: null,
    last_skill: null,
  });
  await open();
  expect(screen.getByText("None or sharing disabled")).toBeTruthy();
  expect(screen.getByText("No retrieval or memory disabled")).toBeTruthy();
  expect(screen.getByText("Not measured")).toBeTruthy();
});
