import { useEffect, useRef, type RefObject } from "react";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import { desktop } from "../api/tauri";

type Point = { x: number; y: number };
type Geometry = { position: Point; scale: number };
type PointerOperation =
  "visibility" | "geometry" | "cursor" | "hit-test" | "ignore" | "listeners";
export interface BuddyPointerAdapter {
  geometry: () => Promise<Geometry>;
  cursor: () => Promise<Point>;
  visible: () => Promise<boolean>;
  ignore: (ignore: boolean) => Promise<void>;
  onGeometryChange: (callback: () => void) => Promise<() => void>;
}

export function toBuddyPoint(cursor: Point, geometry: Geometry): Point {
  return {
    x: (cursor.x - geometry.position.x) / geometry.scale,
    y: (cursor.y - geometry.position.y) / geometry.scale,
  };
}

/** CSS excludes transparent SVG pixels before elementFromPoint resolves a target. */
export function isBuddyTarget(
  root: HTMLElement,
  target: Element | null,
): boolean {
  return !!(
    target &&
    root.contains(target) &&
    target.closest(
      ".desktop-character .avatar :is(path, text, circle, ellipse, rect, polygon, polyline), .buddy-action-surface, .companion-chat, .suggestion-card, .companion-error",
    )
  );
}

/** Global cursor sampling still works while this window ignores mouse input. */
export function startBuddyPointer({
  adapter,
  hitTest,
  viewport,
  held,
  onError,
}: {
  adapter: BuddyPointerAdapter;
  hitTest: (point: Point) => boolean;
  viewport: () => { width: number; height: number };
  held: () => boolean;
  onError: (operation: PointerOperation) => void;
}): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let geometry: Geometry | undefined;
  let geometryDirty = true;
  let revision = 0;
  let ignored = false;
  let nextVisibility = 0;
  let visible = true;
  let unlisten: (() => void) | undefined;
  let failures = 0;
  const restore = async () => {
    // Fail interactive: a failed probe must not leave an unreachable pet.
    await adapter.ignore(false).catch(() => {});
    ignored = false;
  };
  const tick = async () => {
    let delay = 32;
    let operation: PointerOperation = "visibility";
    try {
      if (Date.now() >= nextVisibility) {
        visible = await adapter.visible();
        nextVisibility = Date.now() + 1000;
      }
      if (!visible) {
        delay = 250;
      } else {
        if (geometryDirty || !geometry) {
          operation = "geometry";
          const observedRevision = revision;
          geometry = await adapter.geometry();
          if (!(geometry.scale > 0) || !Number.isFinite(geometry.scale))
            throw new Error("Invalid window scale");
          geometryDirty = observedRevision !== revision;
        }
        operation = "cursor";
        const point = toBuddyPoint(await adapter.cursor(), geometry);
        if (!Number.isFinite(point.x) || !Number.isFinite(point.y))
          throw new Error("Invalid cursor position");
        if (stopped) return;
        operation = "hit-test";
        const interactive = hitTest(point);
        const ignore = !held() && !interactive;
        if (ignore !== ignored) {
          operation = "ignore";
          await adapter.ignore(ignore);
          ignored = ignore;
        }
        const size = viewport();
        if (
          point.x < -80 ||
          point.y < -80 ||
          point.x > size.width + 80 ||
          point.y > size.height + 80
        )
          delay = 160;
      }
      failures = 0;
    } catch {
      await restore();
      if (stopped) return;
      geometryDirty = true;
      nextVisibility = 0;
      failures += 1;
      if (failures >= 5) {
        stopped = true;
        unlisten?.();
        onError(operation);
      } else {
        // A locked desktop or a window transition can briefly reject native probes.
        delay = Math.min(250 * 2 ** (failures - 1), 2000);
      }
    } finally {
      if (stopped) await restore();
      else timer = setTimeout(() => void tick(), delay);
    }
  };
  void (async () => {
    try {
      unlisten = await adapter.onGeometryChange(() => {
        revision += 1;
        geometryDirty = true;
      });
      if (stopped) unlisten();
      else void tick();
    } catch {
      if (!stopped) {
        stopped = true;
        await restore();
        onError("listeners");
      }
    }
  })();
  return () => {
    stopped = true;
    clearTimeout(timer);
    unlisten?.();
    void restore();
  };
}

export function useBuddyPointer(
  root: RefObject<HTMLElement | null>,
  enabled: boolean,
  busy: boolean,
  onError: (operation: PointerOperation) => void,
  onHover: (hovered: boolean) => void,
) {
  const busyRef = useRef(busy);
  const errorRef = useRef(onError);
  const hoverRef = useRef(onHover);
  busyRef.current = busy;
  errorRef.current = onError;
  hoverRef.current = onHover;
  useEffect(() => {
    if (!desktop || !enabled) return;
    const window = getCurrentWindow();
    if (!["buddy", "mascot"].includes(window.label)) return;
    let pressed = false;
    const down = () => {
      pressed = true;
    };
    const up = () => {
      pressed = false;
    };
    document.addEventListener("pointerdown", down, true);
    document.addEventListener("pointerup", up, true);
    document.addEventListener("pointercancel", up, true);
    document.addEventListener("lostpointercapture", up, true);
    globalThis.addEventListener("blur", up);
    const stop = startBuddyPointer({
      adapter: {
        geometry: async () => {
          const [position, scale] = await Promise.all([
            window.innerPosition(),
            window.scaleFactor(),
          ]);
          return { position, scale };
        },
        cursor: cursorPosition,
        visible: () => window.isVisible(),
        ignore: (ignore) => window.setIgnoreCursorEvents(ignore),
        onGeometryChange: async (callback) => {
          const moved = await window.onMoved(callback);
          try {
            const resized = await window.onResized(callback);
            return () => {
              moved();
              resized();
            };
          } catch (error) {
            moved();
            throw error;
          }
        },
      },
      hitTest: ({ x, y }) => {
        const target = document.elementFromPoint(x, y);
        const interactive =
          !!root.current && isBuddyTarget(root.current, target);
        // Restoring OS mouse input need not dispatch pointerenter until the mouse moves again.
        hoverRef.current(
          interactive &&
            !!target?.closest(".desktop-character, .buddy-action-surface"),
        );
        return interactive;
      },
      viewport: () => ({ width: innerWidth, height: innerHeight }),
      held: () => pressed || busyRef.current,
      onError: (operation) => errorRef.current(operation),
    });
    return () => {
      document.removeEventListener("pointerdown", down, true);
      document.removeEventListener("pointerup", up, true);
      document.removeEventListener("pointercancel", up, true);
      document.removeEventListener("lostpointercapture", up, true);
      globalThis.removeEventListener("blur", up);
      stop();
      hoverRef.current(false);
    };
  }, [root, enabled]);
}
