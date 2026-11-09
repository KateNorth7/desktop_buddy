// @vitest-environment jsdom
import { queryClient } from "../data/client";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CoreSettings } from "./Settings";
import { api } from "../api/tauri";
import { serviceStatusApi, type ServiceSnapshot } from "../ai/serviceStatus";
import { coreApi } from "./api";
import { emptyCore } from "./types";
import {
  defaultUserSettings,
  defaultBuddyPreferences,
  defaultTrackingSettings,
  type Dashboard,
} from "../types";

vi.mock("../api/tauri", () => ({
  desktop: true,
  api: {
    trackingSettings: vi.fn(),
    testConnection: vi.fn(),
    providerKey: vi.fn(),
  },
}));
vi.mock("../ai/serviceStatus", () => ({ serviceStatusApi: { read: vi.fn() } }));
vi.mock("./api", () => ({
  coreApi: { snapshot: vi.fn(), preferences: vi.fn() },
}));
vi.mock("../components/MacPermissions", () => ({ MacPermissions: () => null }));
vi.mock("../components/MemorySettings", () => ({ MemorySettings: () => null }));
vi.mock("../companion/CompanionSettings", () => ({
  CompanionSettings: () => null,
}));
vi.mock("../ai/PersonalAiEngine", () => ({
  PersonalAiEnginePanel: () => null,
}));
vi.mock("../ai/AiSettings", () => ({ AiSettings: () => null }));
vi.mock("./TrackingStatus", () => ({ TrackingStatus: () => null }));

const data = {
  status: { dnd: false, anthropic_configured: true, web_search_configured: true },
  user_settings: defaultUserSettings,
  buddy: { preferences: defaultBuddyPreferences },
} as Dashboard;
let snapshot: ServiceSnapshot;
let workspace = emptyCore();
beforeEach(() => {
  queryClient.invalidate(undefined, true);
  snapshot = {
    configured_model: "test-model",
    configured_vision_model: "fixture-screen-model",
    configured_image_description_model: "fixture-image-model",
    services: ["anthropic", "search"].map((id) => ({
      id: id as "anthropic" | "search",
      status: "not_checked",
      checked_at: null,
    })),
  };
  workspace = emptyCore();
  workspace.preferences.ai_connection_mode = "personal";
  vi.mocked(coreApi.snapshot).mockImplementation(async () =>
    structuredClone(workspace),
  );
  vi.mocked(coreApi.preferences).mockResolvedValue(undefined);
  vi.mocked(api.trackingSettings).mockResolvedValue(defaultTrackingSettings);
  vi.mocked(serviceStatusApi.read).mockImplementation(async () =>
    structuredClone(snapshot),
  );
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.resetAllMocks();
});
async function openSettings(onChanged = vi.fn().mockResolvedValue(undefined)) {
  render(
    <CoreSettings
      data={data}
      onChanged={onChanged}
      onDirty={vi.fn()}
      onUpdates={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "AI" }));
  await waitFor(() => expect(serviceStatusApi.read).toHaveBeenCalledOnce());
  return within(screen.getByRole("region", { name: "Services" }));
}

it("defaults the single connection row to built-in AI while keeping personal keys untouched", async () => {
  workspace.preferences.ai_connection_mode = "built_in";
  const services = await openSettings();
  expect(
    services.getByRole("combobox", { name: "AI connection" }),
  ).toHaveProperty("value", "built_in");
  expect(
    services
      .getAllByRole("button", { name: "Replace key" })
      .every((b) => (b as HTMLButtonElement).disabled),
  ).toBe(true);
  fireEvent.click(services.getAllByRole("button", { name: "Test" })[0]);
  await waitFor(() =>
    expect(api.testConnection).toHaveBeenCalledWith("anthropic"),
  );
  expect(api.providerKey).not.toHaveBeenCalled();
});

it("switches connection automatically without replacing, deleting or requesting saved keys", async () => {
  workspace.preferences.ai_connection_mode = "built_in";
  vi.mocked(coreApi.preferences).mockImplementation(async (next) => {
    workspace.preferences = next;
    snapshot = { ...snapshot, connection_mode: next.ai_connection_mode };
  });
  const services = await openSettings();
  const selector = services.getByRole("combobox", { name: "AI connection" });
  await waitFor(() =>
    expect((selector as HTMLSelectElement).disabled).toBe(false),
  );
  fireEvent.change(selector, { target: { value: "personal" } });
  await waitFor(() =>
    expect(coreApi.preferences).toHaveBeenCalledWith(
      expect.objectContaining({ ai_connection_mode: "personal" }),
    ),
  );
  await waitFor(() =>
    expect(
      services
        .getAllByRole("button", { name: "Replace key" })
        .every((b) => !(b as HTMLButtonElement).disabled),
    ).toBe(true),
  );
  expect(api.providerKey).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: "Save preferences" })).toBeNull();
});

it("keeps a successful provider check after the temporary receipt disappears", async () => {
  const services = await openSettings();
  vi.mocked(api.testConnection).mockImplementation(async (provider) => {
    snapshot.services.find((item) => item.id === provider)!.status = "verified";
    return "Connected";
  });
  vi.useFakeTimers();
  await act(async () => {
    fireEvent.click(services.getAllByRole("button", { name: "Test" })[0]);
  });
  await act(async () => {
    vi.advanceTimersByTime(5000);
  });
  expect(services.getAllByRole("status").map((row) => row.textContent)).toEqual(
    ["Connected"],
  );
  expect(serviceStatusApi.read).toHaveBeenCalledTimes(2);
});

it("refreshes failed checks and preserves short errors until the next action", async () => {
  const services = await openSettings();
  vi.mocked(api.testConnection).mockImplementation(async () => {
    snapshot.services[0].status = "unavailable";
    throw new Error("Connection failed");
  });
  vi.useFakeTimers();
  await act(async () => {
    fireEvent.click(services.getAllByRole("button", { name: "Test" })[0]);
  });
  await act(async () => {
    vi.advanceTimersByTime(5000);
  });
  expect(services.getAllByRole("status")[0].textContent).toBe(
    "Connection failed",
  );
  expect(serviceStatusApi.read).toHaveBeenCalledTimes(2);
});

it("does not relabel a verified key as failed when refreshing the dashboard fails", async () => {
  const services = await openSettings(
    vi.fn().mockRejectedValue(new Error("Dashboard unavailable")),
  );
  vi.mocked(api.testConnection).mockImplementation(async () => {
    snapshot.services[0].status = "verified";
    return "Connected";
  });
  vi.useFakeTimers();
  await act(async () => {
    fireEvent.click(services.getAllByRole("button", { name: "Test" })[0]);
  });
  await act(async () => {
    vi.advanceTimersByTime(5000);
  });
  expect(services.getAllByRole("status")[0].textContent).toBe("Connected");
  expect(services.queryByText(/Failed|Dashboard unavailable/)).toBeNull();
});

it("polls local status without probing providers and distinguishes expiry from failure", async () => {
  snapshot.services[0].status = "verified";
  const services = await openSettings();
  await waitFor(() =>
    expect(services.getAllByRole("status")[0].textContent).toBe("Connected"),
  );
  snapshot.services[0].status = "expired";
  await act(async () => {
    fireEvent(window, new Event("focus"));
  });
  await waitFor(() =>
    expect(services.getAllByRole("status")[0].textContent).toBe("Check again"),
  );
  expect(api.testConnection).not.toHaveBeenCalled();
  vi.mocked(serviceStatusApi.read).mockRejectedValue(
    new Error("Local status unavailable"),
  );
  await act(async () => {
    fireEvent(window, new Event("focus"));
  });
  await waitFor(() =>
    expect(services.getAllByRole("status")[0].textContent).toBe(
      "Key saved, not checked yet",
    ),
  );
});

it("shows Vision as a compact service row and saves only explicit model edits", async () => {
  const services = await openSettings();
  await waitFor(() =>
    expect(
      services.getByText("screenshots · fixture-screen-model"),
    ).toBeTruthy(),
  );
  const row = within(services.getByRole("group", { name: "Vision model" }));
  expect(row.getByText("Uses Claude")).toBeTruthy();
  expect(
    row.getByRole("img", { name: /Image descriptions: fixture-image-model/ }),
  ).toBeTruthy();
  expect(
    services.queryByRole("textbox", { name: "Vision model ID" }),
  ).toBeNull();
  expect(coreApi.preferences).not.toHaveBeenCalled();
  fireEvent.click(row.getByRole("button", { name: "Change model" }));
  let field = services.getByRole("textbox", { name: "Vision model ID" });
  expect((field as HTMLInputElement).value).toBe("fixture-screen-model");
  fireEvent.change(field, { target: { value: "cancelled-model" } });
  fireEvent.click(row.getByRole("button", { name: "Cancel" }));
  expect(coreApi.preferences).not.toHaveBeenCalled();
  fireEvent.click(row.getByRole("button", { name: "Change model" }));
  field = services.getByRole("textbox", { name: "Vision model ID" });
  fireEvent.change(field, { target: { value: " custom/vision " } });
  fireEvent.click(services.getByRole("button", { name: "Save model" }));
  await waitFor(() =>
    expect(coreApi.preferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ vision_model: "custom/vision" }),
    ),
  );
  fireEvent.click(row.getByRole("button", { name: "Use default" }));
  await waitFor(() =>
    expect(coreApi.preferences).toHaveBeenLastCalledWith(
      expect.objectContaining({ vision_model: "" }),
    ),
  );
});
