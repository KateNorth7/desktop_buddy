import { describe, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  isTauri: () => true,
}));
import { invoke } from "@tauri-apps/api/core";
import { api, safeUrl } from "./tauri";
describe("provider settings commands", () => {
  it("fetches completed histories or a specific goal without mixing goal IDs", async () => {
    await api.goalHistory();
    expect(invoke).toHaveBeenLastCalledWith("get_goal_history", {
      goalId: null,
    });
    await api.goalHistory(7);
    expect(invoke).toHaveBeenLastCalledWith("get_goal_history", { goalId: 7 });
  });
  it("uses Quiet mode without changing tracking", async () => {
    await api.dnd(true);
    expect(invoke).toHaveBeenLastCalledWith("set_dnd", { enabled: true });
    await api.dnd(false);
    expect(invoke).toHaveBeenLastCalledWith("set_dnd", { enabled: false });
  });
  it("saves an explicit provider key through the native boundary", async () => {
    await api.providerKey("anthropic", "test-key");
    expect(invoke).toHaveBeenLastCalledWith("set_provider_key", {
      provider: "anthropic",
      key: "test-key",
    });
  });
  it("removes a saved key without sending an empty replacement", async () => {
    await api.providerKey("anthropic", null);
    expect(invoke).toHaveBeenLastCalledWith("set_provider_key", {
      provider: "anthropic",
      key: null,
    });
  });
});
describe("search result URLs", () => {
  it("allows web links", () =>
    expect(safeUrl("https://example.com")).toBe("https://example.com/"));
  it.each(["javascript:alert(1)", "file:///secret", "invalid"])(
    "rejects unsafe URL %s",
    (value) => expect(safeUrl(value)).toBeUndefined(),
  );
});
