// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  isBuddyTarget,
  startBuddyPointer,
  toBuddyPoint,
  type BuddyPointerAdapter,
} from "./useBuddyPointer";

vi.mock("../api/tauri", () => ({ desktop: false }));
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
});

function setup() {
  let changed = () => {};
  const unlisten = vi.fn();
  const adapter: BuddyPointerAdapter = {
    geometry: vi
      .fn()
      .mockResolvedValue({ position: { x: -200, y: 100 }, scale: 2 }),
    cursor: vi.fn().mockResolvedValue({ x: -100, y: 200 }),
    visible: vi.fn().mockResolvedValue(true),
    ignore: vi.fn().mockResolvedValue(undefined),
    onGeometryChange: vi.fn(async (callback) => {
      changed = callback;
      return unlisten;
    }),
  };
  const hitTest = vi.fn().mockReturnValue(false);
  const held = vi.fn().mockReturnValue(false);
  const onError = vi.fn();
  const stop = startBuddyPointer({
    adapter,
    hitTest,
    held,
    onError,
    viewport: () => ({ width: 160, height: 152 }),
  });
  return {
    adapter,
    hitTest,
    held,
    onError,
    stop,
    unlisten,
    changed: () => changed(),
  };
}
async function tick(ms = 32) {
  await vi.advanceTimersByTimeAsync(ms);
}

it.each([1, 1.25, 2, 3])(
  "converts physical coordinates on negative-origin monitors at scale %s",
  (scale) => {
    expect(
      toBuddyPoint(
        { x: -1920 + 40 * scale, y: -900 + 50 * scale },
        { position: { x: -1920, y: -900 }, scale },
      ),
    ).toEqual({ x: 40, y: 50 });
  },
);
it("accepts only painted character elements and visible interactive surfaces", () => {
  const root = document.createElement("main");
  root.innerHTML = `<button class="desktop-character"><svg class="avatar"><path/></svg></button><div class="buddy-action-bar"><div class="buddy-action-surface"><button>Talk</button></div></div><div class="companion-chat"><textarea></textarea></div><div class="suggestion-card"><p>Card</p></div>`;
  document.body.append(root);
  for (const selector of [
    "path",
    ".buddy-action-surface button",
    "textarea",
    "p",
  ])
    expect(isBuddyTarget(root, root.querySelector(selector))).toBe(true);
  for (const selector of [".desktop-character", "svg", ".buddy-action-bar"])
    expect(isBuddyTarget(root, root.querySelector(selector))).toBe(false);
  expect(isBuddyTarget(root, document.body)).toBe(false);
  expect(isBuddyTarget(root, null)).toBe(false);
});
it("passes empty areas through, then restores the pet using global cursor sampling", async () => {
  const s = setup();
  await tick(0);
  expect(s.hitTest).toHaveBeenCalledWith({ x: 50, y: 50 });
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(true);
  await tick();
  expect(s.adapter.ignore).toHaveBeenCalledTimes(1);
  s.hitTest.mockReturnValue(true);
  await tick();
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(false);
  s.stop();
});
it("refreshes coordinates after moving or resizing the native window", async () => {
  const s = setup();
  await tick(0);
  vi.mocked(s.adapter.geometry).mockResolvedValue({
    position: { x: -150, y: 150 },
    scale: 1,
  });
  s.changed();
  await tick();
  expect(s.hitTest).toHaveBeenLastCalledWith({ x: 50, y: 50 });
  expect(s.adapter.geometry).toHaveBeenCalledTimes(2);
  s.stop();
});
it("keeps input during a pointer press or native drag", async () => {
  const s = setup();
  s.held.mockReturnValue(true);
  await tick(0);
  expect(s.adapter.ignore).not.toHaveBeenCalled();
  s.held.mockReturnValue(false);
  await tick();
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(true);
  s.stop();
});
it("slows sampling far away and skips cursor probes while hidden", async () => {
  const s = setup();
  vi.mocked(s.adapter.cursor).mockResolvedValue({ x: 3000, y: 3000 });
  await tick(0);
  await tick(159);
  expect(s.adapter.cursor).toHaveBeenCalledTimes(1);
  await tick(1);
  expect(s.adapter.cursor).toHaveBeenCalledTimes(2);
  vi.mocked(s.adapter.visible).mockResolvedValue(false);
  await tick(1000);
  const calls = vi.mocked(s.adapter.cursor).mock.calls.length;
  await tick(500);
  expect(s.adapter.cursor).toHaveBeenCalledTimes(calls);
  s.stop();
});
it("never overlaps slow native probes and restores input if stopped in flight", async () => {
  let resolve!: (point: { x: number; y: number }) => void;
  const s = setup();
  vi.mocked(s.adapter.cursor).mockImplementation(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  await tick(0);
  await tick(1000);
  expect(s.adapter.cursor).toHaveBeenCalledTimes(1);
  s.stop();
  resolve({ x: -100, y: 200 });
  await tick(0);
  expect(s.adapter.ignore).not.toHaveBeenCalledWith(true);
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(false);
  expect(s.unlisten).toHaveBeenCalledTimes(1);
  expect(vi.getTimerCount()).toBe(0);
});
it("restores input and retries a temporary OS probe error", async () => {
  const s = setup();
  await tick(0);
  vi.mocked(s.adapter.cursor).mockRejectedValueOnce(new Error("probe failed"));
  await tick();
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(false);
  expect(s.onError).not.toHaveBeenCalled();
  await tick(250);
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(true);
  expect(s.adapter.geometry).toHaveBeenCalledTimes(2);
  s.stop();
});
it("fails interactive after bounded recovery and identifies the failing probe", async () => {
  const s = setup();
  vi.mocked(s.adapter.cursor).mockRejectedValue(new Error("probe failed"));
  await tick(3750);
  expect(s.adapter.cursor).toHaveBeenCalledTimes(5);
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(false);
  expect(s.onError).toHaveBeenCalledExactlyOnceWith("cursor");
  expect(vi.getTimerCount()).toBe(0);
  s.stop();
});
it("cancels a pending recovery when disposed", async () => {
  const s = setup();
  vi.mocked(s.adapter.cursor).mockRejectedValue(new Error("probe failed"));
  await tick(0);
  s.stop();
  await tick(4000);
  expect(s.adapter.cursor).toHaveBeenCalledTimes(1);
  expect(s.onError).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});
it("restores input if changing ignore mode fails", async () => {
  const s = setup();
  vi.mocked(s.adapter.ignore).mockRejectedValueOnce(
    new Error("permission denied"),
  );
  await tick(0);
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(false);
  expect(s.onError).not.toHaveBeenCalled();
  await tick(250);
  expect(s.adapter.ignore).toHaveBeenLastCalledWith(true);
  s.stop();
});
it("unsubscribes even if disposal precedes native listener registration", async () => {
  const s = setup();
  s.stop();
  await tick(0);
  expect(s.unlisten).toHaveBeenCalledTimes(1);
  expect(s.adapter.cursor).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});
