import { useEffect, useRef, useState, type CSSProperties } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit, listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, desktop, safeUrl } from "../api/tauri";
import type { AvatarState, BuddyView, Goal, GoalPlan } from "../types";
import { defaultUserSettings } from "../types";
import { Avatar } from "./Avatar";
import { defaultCompanionView } from "../companion/types";
import { CompanionChat } from "../companion/CompanionChat";
import { InterventionCard } from "../companion/InterventionCard";
import { interventionState } from "../companion/interventionPresentation";
import { BuddyActionBar } from "../companion/BuddyActionBar";
import { useBuddyPointer } from "../companion/useBuddyPointer";
import {
  useBuddySurface,
  type BuddySurface,
} from "../companion/useBuddySurface";
import { useBuddyActions } from "../companion/useBuddyActions";
import { useChatPose } from "../companion/useChatPose";
import { queryClient } from "../data/client";

export function DesktopBuddy({
  view,
  onDnd,
  goal = null,
  plan = null,
  onChanged = async () => {},
  surface = "combined",
}: {
  view: BuddyView;
  onDnd: () => void;
  goal?: Goal | null;
  plan?: GoalPlan | null;
  onChanged?: () => Promise<void>;
  surface?: BuddySurface;
}) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [chatState, setChatState] = useChatPose(
    "panel",
    surface === "panel" &&
      !!view.companion?.chat_open &&
      !view.companion?.quiet_mode,
  );
  const [remotePose, setRemotePose] = useState<AvatarState | null>(null);
  const [mode, setMode] = useState<"text" | "voice">("text");
  const [responseState, setResponseState] = useState<AvatarState | null>(null);
  const responseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const completionSeen = useRef<number | null>(null);
  useEffect(() => {
    const revision = view.completion_revision;
    if (revision == null) return;
    const previous = completionSeen.current;
    completionSeen.current = revision;
    if (view.companion?.quiet_mode) {
      if (responseTimer.current) clearTimeout(responseTimer.current);
      setResponseState(null);
    } else if (previous !== null && revision > previous) {
      setResponseState("success");
      if (responseTimer.current) clearTimeout(responseTimer.current);
      responseTimer.current = setTimeout(() => setResponseState(null), 1600);
    }
  }, [view.completion_revision, view.companion?.quiet_mode]);
  useEffect(
    () => () => {
      if (responseTimer.current) clearTimeout(responseTimer.current);
    },
    [],
  );
  useEffect(() => {
    if (!desktop || surface !== "panel") return;
    let stopped = false;
    let off: (() => void) | undefined;
    void listen<"text" | "voice">("buddy://chat-mode", ({ payload }) => {
      if (payload === "text" || payload === "voice") setMode(payload);
    })
      .then((unlisten) => {
        if (stopped) unlisten();
        else off = unlisten;
      })
      .catch(() => {});
    return () => {
      stopped = true;
      off?.();
    };
  }, [surface]);
  const pointer = useRef<{ x: number; y: number; dragged: boolean } | null>(
    null,
  );
  const current = view.companion ?? defaultCompanionView;
  const quietMode = !!current.quiet_mode;
  const companion = quietMode
    ? { ...current, chat_open: false, intervention: null, notice: null }
    : current;
  const actions = useBuddyActions(companion.chat_open || busy);
  useEffect(() => {
    if (!companion.chat_open) setMode("text");
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && companion.chat_open && desktop)
        void api
          .closeChat()
          .then(onChanged)
          .catch((e) => setError(String(e)));
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [companion.chat_open, onChanged]);
  const card = !quietMode && (view.suggestion || view.decision);
  const appearance = view.avatar ?? defaultUserSettings.profile.avatar;
  const root = useRef<HTMLElement>(null);
  useBuddySurface(root, surface, view, setRemotePose);
  useBuddyPointer(
    root,
    (surface !== "panel" && appearance.visible) ||
      companion.chat_open ||
      !!companion.intervention ||
      !!card,
    busy,
    (operation) => {
      queryClient.recordError(
        `buddy pointer:${operation}`,
        "Native pointer operation failed after recovery attempts.",
      );
      setError("Click-through is unavailable. Restart sumi to try again.");
    },
    actions.setNativeHovered,
  );
  async function action(work: () => Promise<unknown>, celebrate = false) {
    setBusy(true);
    try {
      setError("");
      await work();
      await onChanged();
      if (celebrate) {
        setResponseState("success");
        if (responseTimer.current) clearTimeout(responseTimer.current);
        responseTimer.current = setTimeout(() => setResponseState(null), 1600);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  if (
    !appearance.visible &&
    !companion.chat_open &&
    !companion.intervention &&
    !card
  )
    return null;
  const state: AvatarState = quietMode
    ? "sleeping"
    : (responseState ??
      (companion.intervention
        ? interventionState(companion.intervention)
        : (remotePose ??
          (companion.chat_open
            ? chatState
            : view.background_thinking
              ? "thinking"
              : companion.state))));
  return (
    <main
      ref={root}
      style={
        {
          "--buddy-tail": `${view.panel?.arrow_offset ?? 32}px`,
          "--mascot-offset-x": `${view.mascot_offset?.[0] ?? 0}px`,
          "--mascot-offset-y": `${view.mascot_offset?.[1] ?? 0}px`,
        } as CSSProperties
      }
      data-surface={surface}
      data-side={view.panel?.side}
      className={`desktop-buddy surface-${surface} ${appearance.visible ? "" : "character-hidden"} ${companion.chat_open ? "chatting" : companion.intervention || card ? "expanded" : "compact"}`}
    >
      {surface !== "mascot" &&
        (companion.chat_open ? (
          <div className="desktop-compact-input">
            <CompanionChat
              view={companion}
              goal={goal}
              plan={plan}
              onChanged={onChanged}
              onState={setChatState}
              compact
              onMove={
                surface === "panel"
                  ? undefined
                  : (event) => {
                      if (
                        event.button !== 0 ||
                        !desktop ||
                        (event.target as Element).closest(
                          "button, input, textarea, a, select",
                        )
                      )
                        return;
                      event.preventDefault();
                      void action(() => getCurrentWindow().startDragging());
                    }
              }
              mode={mode}
              onMode={setMode}
              onClose={() => void action(api.closeChat)}
            />
          </div>
        ) : companion.intervention ? (
          <InterventionCard
            compactPreview={surface === "panel"}
            prompt={companion.intervention}
            busy={busy}
            respond={(response, deadline) =>
              void action(
                () =>
                  api.companionRespond(
                    companion.intervention!.id,
                    response,
                    deadline,
                  ),
                response === "accept" &&
                  [
                    "completion",
                    "goal_completion",
                    "unplanned_completed",
                  ].includes(companion.intervention!.kind),
              )
            }
          />
        ) : (
          card && (
            <section className="suggestion-card" aria-live="polite">
              <span className="eyebrow">
                {view.suggestion
                  ? "A LITTLE HELP FOR YOUR GOAL"
                  : "A LITTLE CHECK-IN"}
              </span>
              <h2>{view.suggestion?.title || "Want a little help?"}</h2>
              <p>{card.reason}</p>
              {view.suggestion && (
                <button
                  onClick={() =>
                    void action(async () => {
                      const url = safeUrl(view.suggestion!.url);
                      if (url) await openUrl(url);
                    })
                  }
                >
                  Open resource ↗
                </button>
              )}
              <div className="settings-actions">
                {view.decision?.id && (
                  <button
                    className="text-button"
                    onClick={() =>
                      void action(() => api.feedback(view.decision!.id!, true))
                    }
                  >
                    This is related
                  </button>
                )}
                <button className="text-button" onClick={onDnd}>
                  Quiet mode
                </button>
              </div>
              <small>Closes after 45 seconds.</small>
            </section>
          )
        ))}
      {surface !== "panel" && appearance.visible && (
        <>
          <button
            {...actions.events}
            className="desktop-character"
            aria-label="Open sumi mini chat"
            title="Click to chat · drag to move · right-click for actions"
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              pointer.current = {
                x: event.clientX,
                y: event.clientY,
                dragged: false,
              };
              event.currentTarget.setPointerCapture?.(event.pointerId);
            }}
            onPointerMove={(event) => {
              const origin = pointer.current;
              if (
                desktop &&
                origin &&
                !origin.dragged &&
                Math.hypot(
                  event.clientX - origin.x,
                  event.clientY - origin.y,
                ) >= 6
              ) {
                origin.dragged = true;
                event.currentTarget.releasePointerCapture?.(event.pointerId);
                void action(() => getCurrentWindow().startDragging());
              }
            }}
            onPointerUp={() => {
              const origin = pointer.current;
              pointer.current = null;
              if (origin && !origin.dragged && desktop)
                void action(() =>
                  companion.chat_open ? api.closeChat() : api.openChat(),
                );
            }}
            onPointerCancel={() => {
              pointer.current = null;
            }}
            onClick={(event) => {
              if (event.detail === 0 && desktop)
                void action(() =>
                  companion.chat_open ? api.closeChat() : api.openChat(),
                );
            }}
            onContextMenu={(event) => {
              event.preventDefault();
              if (desktop) void action(api.companionMenu);
            }}
          >
            <Avatar
              appearance={{ ...appearance, visible: true }}
              state={state}
            />
          </button>
          <BuddyActionBar
            color={appearance.color}
            actions={actions}
            onWrite={() => {
              setMode("text");
              if (desktop)
                void action(async () => {
                  if (surface === "mascot")
                    await emit("buddy://chat-mode", "text");
                  await api.openChat();
                });
            }}
            onTalk={() => {
              setMode("voice");
              if (desktop)
                void action(async () => {
                  if (surface === "mascot")
                    await emit("buddy://chat-mode", "voice");
                  await api.openChat();
                });
            }}
          />
        </>
      )}
      {!quietMode && error && (
        <small className="companion-error" role="alert">
          {error}
        </small>
      )}
    </main>
  );
}
