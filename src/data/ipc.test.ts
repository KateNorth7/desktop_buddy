// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke as nativeInvoke } from "@tauri-apps/api/core";
import { invoke } from "./ipc";
import { queryClient } from "./client";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
let stop: (() => void) | undefined;
beforeEach(() => {
  vi.useFakeTimers();
  queryClient.invalidate(undefined, true);
  vi.mocked(nativeInvoke).mockResolvedValue({});
});
afterEach(async () => {
  stop?.();
  stop = undefined;
  await vi.advanceTimersByTimeAsync(0);
  vi.useRealTimers();
  vi.clearAllMocks();
});
it("invalidates committed mutations, including partial failure, without a read-triggered refresh loop", async () => {
  const read = vi.fn().mockResolvedValue(1);
  const query = queryClient.query("ipc-workspace", ["workspace"], 0, read);
  stop = query.subscribe(() => {});
  await vi.advanceTimersByTimeAsync(20);
  expect(read).toHaveBeenCalledOnce();
  await invoke("get_core_snapshot", { anchor: null, period: "week" });
  await vi.advanceTimersByTimeAsync(20);
  expect(read).toHaveBeenCalledOnce();
  await invoke("complete_goal_title", { prefix: "L" });
  await invoke("complete_goal_step", { goalId: 7, prefix: "D" });
  await vi.advanceTimersByTimeAsync(20);
  expect(read).toHaveBeenCalledOnce();
  vi.mocked(nativeInvoke).mockRejectedValueOnce("backend action failed");
  await expect(
    invoke("set_core_today", { id: 7, include: true }),
  ).rejects.toBeTruthy();
  await vi.advanceTimersByTimeAsync(20);
  expect(read).toHaveBeenCalledTimes(2);
});
it("never records provider-key arguments in Diagnostics", async () => {
  vi.mocked(nativeInvoke).mockRejectedValueOnce(
    "provider detail: token=also-secret",
  );
  await expect(
    invoke("set_provider_key", {
      provider: "anthropic",
      key: "never-record-this-key",
    }),
  ).rejects.toBeTruthy();
  const details = JSON.stringify(queryClient.getDiagnostics());
  expect(details).toContain("provider detail");
  expect(details).not.toContain("never-record-this-key");
  expect(details).not.toContain("also-secret");
});
