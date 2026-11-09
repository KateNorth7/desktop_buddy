// @vitest-environment jsdom
import { queryClient } from "../data/client";
import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SidebarAiServices } from "./SidebarAiServices";
import { servicesQuery } from "../data/resources";
import { serviceStatusApi, type ServiceSnapshot } from "../ai/serviceStatus";
import { api } from "../api/tauri";
vi.mock("../api/tauri", () => ({
  desktop: true,
  api: { testConnection: vi.fn() },
}));
vi.mock("../ai/serviceStatus", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../ai/serviceStatus")>()),
  serviceStatusApi: { read: vi.fn() },
}));
const fixture: ServiceSnapshot = {
  configured_model: "claude-opus-5-5",
  configured_response_model: "claude-opus-5-5",
  services: ["anthropic", "model", "search"].map((id) => ({
    id: id as "anthropic" | "model" | "search",
    status: "not_checked",
    checked_at: null,
  })),
};
beforeEach(() => {
  queryClient.invalidate(undefined, true);
  vi.resetAllMocks();
  vi.mocked(serviceStatusApi.read).mockResolvedValue(fixture);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("startup probes the provider without claiming health from key metadata alone", async () => {
  render(<SidebarAiServices />);
  await screen.findByRole("button", { name: "Claude model: Not checked" });
  expect(api.testConnection).toHaveBeenCalledTimes(1);
  expect(document.querySelectorAll(".service-verified")).toHaveLength(0);
});
it("rechecks the services after the selected connection changes", async () => {
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    connection_mode: "built_in",
  });
  render(<SidebarAiServices />);
  await waitFor(() => expect(api.testConnection).toHaveBeenCalledTimes(1));
  await act(async () => {
    await Promise.resolve();
  });
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    connection_mode: "personal",
  });
  await act(async () => {
    servicesQuery.setData({ ...fixture, connection_mode: "personal" });
  });
  await waitFor(() => expect(api.testConnection).toHaveBeenCalledTimes(2));
  expect(document.querySelectorAll(".service-verified")).toHaveLength(0);
});
it("hover details explain the cloud model and preserve configured-model provenance", async () => {
  const user = userEvent.setup();
  render(<SidebarAiServices />);
  await user.hover(
    await screen.findByRole("button", { name: "Claude model: Not checked" }),
  );
  const tooltip = screen.getByRole("dialog", {
    name: "Claude model connection",
  });
  expect(tooltip.textContent).toContain(
    "Runs through Claude, not on this computer",
  );
  // Model identifiers moved to Settings · AI; the popover keeps the plain explanation.
  expect(tooltip.textContent).not.toContain("Configured model:");
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
});
it("manual refresh checks both providers again after startup", async () => {
  const user = userEvent.setup();
  render(<SidebarAiServices />);
  await screen.findByRole("button", { name: "Claude model: Not checked" });
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({
      ...s,
      status: "verified",
      checked_at: new Date().toISOString(),
    })),
  });
  await user.click(
    screen.getByRole("button", { name: "Check AI connections" }),
  );
  await screen.findByRole("button", { name: "Claude model: Model available" });
  expect(api.testConnection).toHaveBeenCalledTimes(2);
  expect(api.testConnection).toHaveBeenCalledWith("anthropic");
  expect(screen.getByRole("status").textContent).toContain("refreshed");
});
it("a failed probe still refreshes status and never treats failure as success", async () => {
  const user = userEvent.setup();
  render(<SidebarAiServices />);
  await screen.findByRole("button", { name: "Claude model: Not checked" });
  vi.mocked(api.testConnection).mockRejectedValue(
    new Error("Private provider response"),
  );
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({ ...s, status: "unavailable" })),
  });
  await user.click(
    screen.getByRole("button", { name: "Check AI connections" }),
  );
  await screen.findByRole("button", {
    name: "Claude: Could not verify access",
  });
  expect(document.body.textContent).not.toContain("Private provider response");
  expect(document.querySelectorAll(".service-verified")).toHaveLength(0);
});
it("offline immediately overrides a previously verified connection", async () => {
  const online = vi.spyOn(navigator, "onLine", "get").mockReturnValue(true);
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({ ...s, status: "verified" })),
  });
  render(<SidebarAiServices />);
  await screen.findByRole("button", { name: "Claude: Connected" });
  online.mockReturnValue(false);
  fireEvent(window, new Event("offline"));
  await screen.findByRole("button", { name: "Claude: Offline" });
  expect(
    (
      screen.getByRole("button", {
        name: "Check AI connections",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
});
it("status read failure clears green rather than retaining a successful connection", async () => {
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({ ...s, status: "verified" })),
  });
  render(<SidebarAiServices />);
  await screen.findByRole("button", { name: "Claude: Connected" });
  vi.mocked(serviceStatusApi.read).mockRejectedValue(new Error("Unavailable"));
  fireEvent(window, new Event("focus"));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Claude: Not checked" }),
    ).toBeTruthy(),
  );
});
it("onboarding performs no diagnostics or provider requests", () => {
  render(<SidebarAiServices enabled={false} />);
  expect(serviceStatusApi.read).not.toHaveBeenCalled();
  expect(api.testConnection).not.toHaveBeenCalled();
});

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
it("startup stays neutral until the real probe finishes and web search follows the Claude connection", async () => {
  const anthropic = deferred();
  vi.mocked(api.testConnection).mockImplementation(() =>
    anthropic.promise.then(() => "Checked"),
  );
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({ ...s, status: "not_configured" })),
  });
  render(<SidebarAiServices />);
  await screen.findByRole("button", { name: "Claude: Checking…" });
  expect(
    screen.getByRole("button", { name: "Web search: Checking…" }),
  ).toBeTruthy();
  await waitFor(() => expect(api.testConnection).toHaveBeenCalledTimes(1));
  expect(api.testConnection).toHaveBeenCalledWith("anthropic");
  expect(document.querySelectorAll(".service-not_configured")).toHaveLength(0);
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({
      ...s,
      status: s.id === "search" ? "unavailable" : "verified",
    })),
  });
  await act(async () => anthropic.resolve());
  await screen.findByRole("button", { name: "Claude: Connected" });
  await screen.findByRole("button", {
    name: "Web search: Could not verify access",
  });
});
it("StrictMode and sidebar collapse do not duplicate startup probes", async () => {
  const view = render(
    <StrictMode>
      <SidebarAiServices />
    </StrictMode>,
  );
  await screen.findByRole("button", { name: "Claude: Not checked" });
  view.rerender(
    <StrictMode>
      <SidebarAiServices collapsed />
    </StrictMode>,
  );
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(serviceStatusApi.read).toHaveBeenCalled());
  expect(api.testConnection).toHaveBeenCalledTimes(1);
});
it("an offline startup waits for reconnection before probing", async () => {
  const online = vi.spyOn(navigator, "onLine", "get").mockReturnValue(false);
  render(<SidebarAiServices />);
  expect(api.testConnection).not.toHaveBeenCalled();
  online.mockReturnValue(true);
  fireEvent(window, new Event("online"));
  await screen.findByRole("button", { name: "Claude: Not checked" });
  expect(api.testConnection).toHaveBeenCalledTimes(1);
});
it("simulated providers never trigger real startup network checks", async () => {
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({ ...s, status: "mock" })),
  });
  render(<SidebarAiServices />);
  await screen.findByRole("button", { name: "Claude: Simulated provider" });
  expect(api.testConnection).not.toHaveBeenCalled();
});
it("disabling before metadata arrives prevents startup probes", async () => {
  const pending = deferred();
  vi.mocked(serviceStatusApi.read).mockImplementation(async () => {
    await pending.promise;
    return fixture;
  });
  const view = render(<SidebarAiServices />);
  view.rerender(<SidebarAiServices enabled={false} />);
  await act(async () => pending.resolve());
  expect(api.testConnection).not.toHaveBeenCalled();
});

it("refreshes expired connections directly from a status-dot card", async () => {
  const user = userEvent.setup();
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({ ...s, status: "expired" })),
  });
  render(<SidebarAiServices variant="dots" />);
  await user.click(
    await screen.findByRole("button", { name: "Claude: Check again" }),
  );
  const refresh = screen.getByRole("button", {
    name: "Refresh Claude connection",
  });
  vi.mocked(serviceStatusApi.read).mockResolvedValue({
    ...fixture,
    services: fixture.services.map((s) => ({ ...s, status: "verified" })),
  });
  await user.click(refresh);
  await screen.findByRole("button", { name: "Claude: Connected" });
  expect(api.testConnection).toHaveBeenCalledTimes(2);
});
