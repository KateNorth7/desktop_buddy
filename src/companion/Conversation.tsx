import { InterventionCard } from "./InterventionCard";
import { CoachingPanel } from "../coaching/CoachingPanel";
import { Markdown } from "./Markdown";
import { MessageMeta, Sources } from "./MessageParts";
import { useQuery } from "../data/useQuery";
import {
  chatHistoryQuery,
  preferencesQuery,
  coachingQuery,
} from "../data/resources";
import { queryClient } from "../data/client";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import "./conversation.css";
import {
  ArrowUpRight,
  Mic,
  MicOff,
  Paperclip,
  Send,
  Square,
  SquarePen,
  Trash2,
  X,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { api, desktop, safeUrl } from "../api/tauri";
import { aiApi, type ChatMessage } from "../ai/api";
import { readAttachment, recordVoice } from "../core/media";
import { coreApi } from "../core/api";
import type { AvatarState, Goal, GoalPlan } from "../types";
import type { CompanionView } from "./types";
import { type CoachingView } from "../coaching/api";

function chatError(error: unknown) {
  queryClient.recordError("chat action", error);
  const local = [
    "Connect or replace the provider key in Settings → Integrations / AI, then retry.",
    "Check your provider access and credits in Settings, then retry.",
    "sumi's provider is busy. Wait a moment and retry; your draft is kept.",
    "sumi's AI provider took too long to respond or could not be reached. Your message is saved. Check the chat and your goals before retrying.",
    "sumi's AI provider is temporarily unavailable. Your message is saved; try again in a moment.",
    "sumi's AI provider rejected this request. Your message is saved. Try a new chat or update sumi before retrying.",
    "Your goal or sharing choices changed. Retry with the current context.",
    "Check that your provider endpoint and model are supported, then retry. Your draft is kept.",
    "AI assistance is paused. Enable it in Settings → Integrations / AI to continue.",
    "sumi is finishing another request. Try again in a moment.",
    "sumi could not prepare a reliable answer. Retry or describe your request more specifically; your draft is kept.",
    "No speech was recognized. Try again or type your message.",
    "The transcript is too long. Record a shorter message.",
    "Local speech recognition is unavailable. Install an on-device speech language or type your message.",
    "This attachment needs vision. Use Import to review an image or scanned PDF.",
    "The speech model could not download. Check your connection and try again.",
    "The speech model download is unavailable. Try again later.",
    "The speech model download stopped. Try again.",
    "The speech model failed its integrity check. Try again.",
    "Could not save the speech model. Check free disk space.",
    "Could not load the local speech model. Close other large apps and try again.",
    "Local speech recognition failed. Try again or type your message.",
    "Download the on-device speech model before recording.",
    "Another recording is being transcribed. Try again shortly.",
  ];
  return (
    local.find(
      (message) =>
        (error instanceof Error ? error.message : String(error)) === message,
    ) ??
    "sumi couldn't finish this request. Check the chat and your goals before retrying."
  );
}

type PendingMessage = {
  text: string;
  attachment: string | null;
  voiceMessage: boolean;
  requestId: string;
  goalId: number | null;
  coachingSessionId: number | null;
  coachingRevision: number | null;
  rememberResult: boolean;
};

export function Conversation({
  view,
  goal,
  plan,
  onChanged,
  onState,
  embedded = false,
  headerless = false,
  onMove,
  onResize,
  compact = false,
  mode = "text",
  onMode,
  onClose,
  conversationId,
}: {
  view: CompanionView;
  goal: Goal | null;
  plan: GoalPlan | null;
  onChanged: () => Promise<void>;
  onState: (state: AvatarState) => void;
  embedded?: boolean;
  headerless?: boolean;
  onMove?: (event: React.PointerEvent<HTMLElement>) => void;
  onResize?: () => void;
  compact?: boolean;
  mode?: "text" | "voice";
  onMode?: (mode: "text" | "voice") => void;
  onClose?: () => void;
  /** The chat this view shows; replies are saved there even if another chat is opened meanwhile. */
  conversationId?: number;
}) {
  const inputId = useId();
  const [text, setText] = useState(view.seed.slice(0, 4000));
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const historyQuery = chatHistoryQuery(conversationId);
  const history = useQuery(historyQuery, desktop);
  const prefs = useQuery(preferencesQuery, desktop);
  const [attachment, setAttachment] = useState<{
    name: string;
    text: string;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [muted, setMuted] = useState(false);
  const [recording, setRecording] = useState(false);
  const [voicePreparation, setVoicePreparation] = useState("");
  const [voiceProcessing, setVoiceProcessing] = useState("");
  const [voiceOriginal, setVoiceOriginal] = useState<string | null>(null);
  const [voiceCleanupUnavailable, setVoiceCleanupUnavailable] = useState(false);
  const preparingVoice = useRef<number | null>(null);
  const voicePreparationStops = useRef(new Set<() => void>());
  const recorder = useRef<Awaited<ReturnType<typeof recordVoice>> | null>(null);
  const voiceGeneration = useRef(0);
  const voiceDraft = useRef(false);
  const voiceTimeout = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const sending = useRef(false);
  const queued = useRef<PendingMessage[]>([]);
  const [waiting, setWaiting] = useState(false);
  const [coaching, setCoaching] = useState<CoachingView | null>(null);
  const [rememberResult, setRememberResult] = useState(false);
  const updateCoaching = useCallback((next: CoachingView | null) => {
    setCoaching((previous) =>
      next &&
      previous &&
      (previous.session.id > next.session.id ||
        (previous.session.id === next.session.id &&
          previous.session.revision > next.session.revision))
        ? previous
        : next,
    );
  }, []);
  const followLatest = useRef(true);
  // Voice callbacks outlive a render, so they read the latest values from here.
  const latest = useRef({ text });
  latest.current = { text };
  function cleanupEnabled() {
    const policy = preferencesQuery.getSnapshot();
    return (
      policy.ready &&
      !policy.error &&
      policy.data.enabled &&
      !!policy.data.voice_text_cleanup
    );
  }
  const cancelRecording = () => {
    voiceGeneration.current += 1;
    recorder.current?.cancel();
    recorder.current = null;
    if (voiceTimeout.current) clearTimeout(voiceTimeout.current);
    setRecording(false);
    setMuted(false);
    setVoicePreparation("");
    setVoiceProcessing("");
    voicePreparationStops.current.forEach((stop) => stop());
    voicePreparationStops.current.clear();
    onState("idle");
  };
  async function finishRecording() {
    const active = recorder.current;
    if (!active) return;
    const generation = ++voiceGeneration.current;
    if (voiceTimeout.current) clearTimeout(voiceTimeout.current);
    recorder.current = null;
    setRecording(false);
    setMuted(false);
    onState("thinking");
    let transcript = "";
    let original = "";
    let tidied = false;
    setVoiceOriginal(null);
    setVoiceCleanupUnavailable(false);
    setVoiceProcessing("Transcribing on this device…");
    const recognized = await run(async () => {
      transcript = (await coreApi.transcribe(await active.stop())).trim();
      if (!transcript)
        throw new Error(
          "No speech was recognized. Try again or type your message.",
        );
      if (
        [latest.current.text, transcript].filter(Boolean).join(" ").length >
        4000
      )
        throw new Error(
          "The transcript is too long. Record a shorter message.",
        );
      original = transcript;
      if (generation !== voiceGeneration.current) return;
      if (cleanupEnabled()) {
        setVoiceProcessing("Tidying your words…");
        try {
          const draft = await coreApi.correctVoiceDraft(transcript);
          if (generation !== voiceGeneration.current) return;
          if (
            cleanupEnabled() &&
            draft.text.trim() &&
            [latest.current.text, draft.text].filter(Boolean).join(" ")
              .length <= 4000
          ) {
            transcript = draft.text;
            tidied = draft.corrected && transcript !== original;
            setVoiceCleanupUnavailable(draft.unavailable);
          } else if (cleanupEnabled()) {
            setVoiceCleanupUnavailable(true);
          }
        } catch (error) {
          queryClient.recordError("Voice text cleanup", error);
          if (generation === voiceGeneration.current)
            setVoiceCleanupUnavailable(true);
        }
      }
    });
    if (generation !== voiceGeneration.current) return;
    setVoiceProcessing("");
    if (!recognized || !transcript) {
      onState("idle");
      return;
    }
    voiceDraft.current = true;
    const combined = [latest.current.text, transcript]
      .filter(Boolean)
      .join(" ")
      .slice(0, 4000);
    setText(combined);
    if (tidied)
      setVoiceOriginal(
        [latest.current.text, original].filter(Boolean).join(" "),
      );
    onMode?.("text");
    input.current?.focus();
    onState("idle");
  }
  async function startRecording() {
    if (
      !desktop ||
      recorder.current ||
      busy ||
      preparingVoice.current === voiceGeneration.current
    )
      return;
    const generation = ++voiceGeneration.current;
    preparingVoice.current = generation;
    setVoiceOriginal(null);
    setVoiceCleanupUnavailable(false);
    setVoicePreparation("Preparing on-device speech…");
    setError("");
    try {
      if (!preferencesQuery.getSnapshot().ready) {
        await preferencesQuery.refresh().catch(() => undefined);
      }
      if (generation !== voiceGeneration.current) return;
      const languages = await coreApi.voiceLanguages();
      if (generation !== voiceGeneration.current) return;
      if (!languages.length)
        throw new Error(
          "Local speech recognition is unavailable. Install an on-device speech language or type your message.",
        );
      const stopProgress = await listen<{ downloaded: number; total: number }>(
        "buddy://voice-model-progress",
        ({ payload }) => {
          if (generation === voiceGeneration.current && payload.total > 0)
            setVoicePreparation(
              `Downloading on-device speech model… ${Math.min(100, Math.round((payload.downloaded / payload.total) * 100))}%`,
            );
        },
      );
      if (generation !== voiceGeneration.current) {
        stopProgress();
        return;
      }
      voicePreparationStops.current.add(stopProgress);
      try {
        await coreApi.prepareVoice();
      } finally {
        stopProgress();
        voicePreparationStops.current.delete(stopProgress);
      }
      if (generation !== voiceGeneration.current) return;
      const active = await recordVoice();
      if (generation !== voiceGeneration.current) {
        active.cancel();
        return;
      }
      recorder.current = active;
      setRecording(true);
      onState("listening");
      voiceTimeout.current = setTimeout(() => void finishRecording(), 60000);
    } catch (e) {
      if (generation === voiceGeneration.current) {
        setError(
          e instanceof DOMException &&
            ["NotAllowedError", "SecurityError"].includes(e.name)
            ? "Microphone access was denied. Allow microphone access in system settings, then try again or type your message."
            : chatError(e),
        );
        onState("idle");
      }
    } finally {
      if (preparingVoice.current === generation) preparingVoice.current = null;
      if (generation === voiceGeneration.current) setVoicePreparation("");
    }
  }
  useEffect(() => {
    const preparationStops = voicePreparationStops.current;
    if (compact && mode === "voice") void startRecording();
    return () => {
      voiceGeneration.current += 1;
      recorder.current?.cancel();
      recorder.current = null;
      if (voiceTimeout.current) clearTimeout(voiceTimeout.current);
      setRecording(false);
      setMuted(false);
      setVoicePreparation("");
      setVoiceProcessing("");
      preparationStops.forEach((stop) => stop());
      preparationStops.clear();
      onState("idle");
    };
    // Recording starts and stops only when the compact voice mode is entered or left;
    // depending on the latest onState/startRecording would restart recording on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [compact, mode]);
  useEffect(() => {
    if (compact && mode === "text") input.current?.focus();
  }, [compact, mode]);
  useEffect(() => {
    if (!embedded && !busy) input.current?.focus();
  }, [embedded, busy]);
  useEffect(() => {
    if (history.ready && !sending.current && !waiting)
      setMessages(history.data);
  }, [history.data, history.ready, waiting]);
  useEffect(() => {
    if (embedded) return;
    setText(view.seed.slice(0, 4000));
    setVoiceOriginal(null);
    setVoiceCleanupUnavailable(false);
    setError("");
  }, [view.seed, view.intent, embedded]);
  useEffect(() => {
    if (scroll.current && followLatest.current)
      scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [messages, busy]);
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (!embedded && !compact && event.key === "Escape") {
        event.preventDefault();
        void api
          .closeChat()
          .then(onChanged)
          .catch((e) => setError(chatError(e)));
      }
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [onChanged, embedded, compact]);
  async function run(work: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await work();
      await onChanged();
      return true;
    } catch (e) {
      setError(chatError(e));
      return false;
    } finally {
      setBusy(false);
    }
  }
  // A message leaves the field and appears in the chat right away. While sumi is still answering,
  // further messages are queued and sent one after another, so typing never has to wait.
  async function submit(prefix = "", override?: string) {
    const body = (override ?? text).trim();
    if (!body || !desktop || (coaching?.chat_bound && sending.current)) return;
    const outgoing = prefix + body;
    const voiceMessage = voiceDraft.current;
    voiceDraft.current = false;
    setVoiceOriginal(null);
    setVoiceCleanupUnavailable(false);
    const shared = attachment?.text ?? null;
    const pending: PendingMessage = {
      text: outgoing,
      attachment: shared,
      voiceMessage,
      requestId: crypto.randomUUID(),
      goalId: coaching?.chat_bound
        ? coaching.session.goal_id
        : (goal?.id ?? null),
      coachingSessionId: coaching?.chat_bound ? coaching.session.id : null,
      coachingRevision: coaching?.chat_bound ? coaching.session.revision : null,
      rememberResult,
    };
    setText("");
    setAttachment(null);
    setError("");
    followLatest.current = true;
    setMessages((previous) => [
      ...previous,
      {
        id: -Date.now(),
        role: "user",
        text: outgoing,
        created_at: new Date().toISOString(),
        goal_id: pending.goalId,
        resources: [],
      },
    ]);
    if (sending.current) {
      queued.current.push(pending);
      return;
    }
    sending.current = true;
    setWaiting(true);
    onState("thinking");
    let next: PendingMessage | undefined = pending;
    try {
      while (next) {
        await aiApi.send(
          next.text,
          next.attachment,
          next.goalId,
          conversationId ?? null,
          next.voiceMessage,
          next.requestId,
          next.coachingSessionId,
          next.coachingRevision,
          next.rememberResult,
        );
        next = queued.current.shift();
      }
    } catch (e) {
      setError(chatError(e));
      // The question stays in the chat once it was accepted; anything else goes back to the input.
      const saved = await historyQuery.refresh().catch(() => []);
      const unsent = [next?.text, ...queued.current.map((item) => item.text)]
        .filter(
          (item): item is string =>
            !!item &&
            !saved.some((m) => m.role === "user" && m.text.startsWith(item)),
        )
        .join("\n");
      queued.current = [];
      if (unsent) setText((previous) => previous || unsent);
    }
    try {
      setMessages(await historyQuery.refresh());
      if (coaching?.chat_bound)
        updateCoaching(await coachingQuery(conversationId).refresh());
      setRememberResult(false);
      await onChanged();
    } catch (e) {
      setError(chatError(e));
    }
    sending.current = false;
    setWaiting(false);
    onState("idle");
    input.current?.focus();
  }
  async function attach(file: File | undefined) {
    if (!file) return;
    await run(async () => {
      const result = await readAttachment(file);
      if (result.images.length)
        throw new Error(
          "This attachment needs vision. Use Import to review an image or scanned PDF.",
        );
      setAttachment({ name: file.name, text: result.text });
    });
    if (fileInput.current) fileInput.current.value = "";
  }
  async function openResource(url: string, goalId: number | null) {
    const safe = safeUrl(url);
    if (!safe) return;
    await run(async () => {
      await openUrl(safe);
      await aiApi.resourceOpened(goalId, safe);
    });
  }
  return (
    <section
      className={`companion-chat ${embedded ? "workspace-chat" : ""} ${compact ? "compact-conversation" : ""} ${compact && mode === "voice" ? "voice-mode" : ""}`}
      aria-label={embedded ? "sumi chat" : "sumi mini chat"}
    >
      {!compact && !headerless && (
        <header onPointerDown={onMove}>
          <div>{!embedded && <strong>sumi</strong>}</div>
          <button
            className="companion-icon"
            aria-label="Clear conversation"
            disabled={!desktop || busy || !messages.length}
            onClick={() => setConfirmClear(true)}
          >
            <Trash2 size={15} />
          </button>
          {!embedded && (
            <button
              className="text-button"
              title="Open full app"
              onClick={() => void run(api.openWorkspace)}
            >
              <ArrowUpRight size={15} />
              Open app
            </button>
          )}
          {!embedded && (
            <button
              className="companion-icon"
              aria-label="Close mini chat"
              onClick={() => void run(api.closeChat)}
            >
              <X size={16} />
            </button>
          )}
        </header>
      )}
      {compact && (
        <header className="compact-header" onPointerDown={onMove}>
          <strong>sumi</strong>
          <button
            type="button"
            className="companion-icon"
            aria-label="Close chat"
            title="Close chat"
            onClick={() => {
              cancelRecording();
              onClose?.();
            }}
          >
            <X size={16} />
          </button>
        </header>
      )}
      {!view.quiet_mode && view.intervention && (
        <InterventionCard
          prompt={view.intervention}
          busy={busy}
          respond={(action, deadline) => {
            void run(async () => {
              await api.companionRespond(
                view.intervention!.id,
                action,
                deadline,
              );
              if (
                action === "accept" &&
                [
                  "completion",
                  "goal_completion",
                  "unplanned_completed",
                ].includes(view.intervention!.kind)
              )
                onState("success");
            });
          }}
        />
      )}
      {
        <div
          className="companion-chat-scroll conversation-history"
          ref={scroll}
          onScroll={(event) => {
            const node = event.currentTarget;
            followLatest.current =
              node.scrollHeight - node.scrollTop - node.clientHeight < 64;
          }}
          role="log"
          aria-label="Conversation history"
          aria-live="polite"
        >
          {!messages.length && (
            <p className="conversation-empty">
              {compact
                ? "Ask sumi anything, or tell it what to add."
                : "What is getting in the way? Tell sumi what you need."}
            </p>
          )}
          {messages.map((message) => (
            <article
              className={`conversation-message ${message.role}`}
              key={message.id}
            >
              <span className="sr-only">
                {message.role === "user" ? "You" : "sumi"}
              </span>
              {message.role === "assistant" ? (
                <Markdown text={message.text} />
              ) : (
                <p>{message.text}</p>
              )}
              <Sources
                resources={message.resources}
                disabled={busy}
                onOpen={(url) => void openResource(url, message.goal_id)}
              />
              <MessageMeta message={message} />
            </article>
          ))}
          {(busy || waiting) && (
            <p role="status">{waiting ? "sumi is thinking…" : "Working…"}</p>
          )}
        </div>
      }
      {embedded && !compact && (
        <CoachingPanel
          conversationId={conversationId}
          onChange={updateCoaching}
          onRememberChange={setRememberResult}
          allowStart={false}
        />
      )}
      {!compact && view.inbox.length > 0 && (
        <details className="conversation-context" aria-label="Saved for later">
          <summary>
            Saved for later ({view.inbox.filter((item) => !item.done).length})
          </summary>
          {view.inbox.map((item) => (
            <div className="conversation-inbox-item" key={item.id}>
              <span>
                {item.text}
                {item.done ? " · Done" : ""}
              </span>
              {!item.done && (
                <button
                  className="text-button"
                  disabled={busy}
                  onClick={() =>
                    void run(() => api.companionInbox(item.id, "complete"))
                  }
                >
                  Done
                </button>
              )}
              <button
                className="text-button"
                disabled={busy}
                onClick={() =>
                  void run(() => api.companionInbox(item.id, "remove"))
                }
              >
                Remove
              </button>
            </div>
          ))}
        </details>
      )}
      {confirmClear && (
        <div className="conversation-confirm" role="alert">
          <p>Clear the saved conversation on this PC?</p>
          <button
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await aiApi.clearHistory();
                setMessages([]);
                setConfirmClear(false);
              })
            }
          >
            Clear history
          </button>
          <button
            className="text-button"
            onClick={() => setConfirmClear(false)}
          >
            Cancel
          </button>
        </div>
      )}
      {(error || history.error || prefs.error || view.notice) && (
        <p className="companion-error" role="alert">
          {error || history.error || prefs.error || view.notice}
          {error && text.trim() && (
            <button
              type="button"
              className="text-button"
              disabled={busy || !desktop}
              onClick={() => void submit()}
            >
              Retry message
            </button>
          )}
        </p>
      )}
      <form
        className="conversation-composer"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        {attachment && (
          <div className="conversation-attachment">
            <span>{attachment.name}</span>
            <button
              type="button"
              className="companion-icon"
              aria-label="Remove attachment"
              onClick={() => setAttachment(null)}
            >
              <X size={13} />
            </button>
          </div>
        )}
        <label htmlFor={inputId} className="sr-only">
          Your message or task
        </label>
        <textarea
          id={inputId}
          ref={input}
          value={text}
          maxLength={4000}
          rows={compact ? 2 : 3}
          disabled={busy || (!!coaching?.chat_bound && waiting)}
          placeholder={
            coaching?.chat_bound
              ? `Reply about ${coaching.target?.title ?? "this goal"}…`
              : "Message sumi…"
          }
          onChange={(e) => {
            setText(e.target.value);
            setVoiceOriginal(null);
            setVoiceCleanupUnavailable(false);
            if (!e.target.value.trim()) voiceDraft.current = false;
          }}
          onKeyDown={(e) => {
            if (
              !e.nativeEvent.isComposing &&
              e.key === "Enter" &&
              !e.shiftKey
            ) {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <div className="companion-composer-actions">
          <input
            ref={fileInput}
            type="file"
            accept=".txt,.md,.pdf"
            hidden
            onChange={(e) => void attach(e.target.files?.[0])}
          />
          {(!compact || mode === "text") && (
            <button
              type="button"
              className="companion-icon"
              aria-label="Attach text or PDF"
              disabled={!desktop || busy || !!coaching?.chat_bound}
              onClick={() => fileInput.current?.click()}
            >
              <Paperclip size={16} />
            </button>
          )}
          <button
            type="button"
            className="companion-icon"
            aria-label={
              compact
                ? "Start voice input"
                : recording
                  ? "Stop voice recording"
                  : "Start voice recording"
            }
            title="Record and transcribe locally"
            disabled={!desktop || busy}
            onClick={() => {
              if (compact) {
                if (mode === "voice") {
                  recorder.current?.setMuted(!muted);
                  setMuted(!muted);
                  onState(muted ? "listening" : "idle");
                } else onMode?.("voice");
                return;
              }
              if (recording) void finishRecording();
              else void startRecording();
            }}
          >
            {compact && muted ? <MicOff size={16} /> : <Mic size={16} />}
          </button>
          {!compact && <span>Enter to send · Shift+Enter for a new line</span>}
          <button
            type="submit"
            disabled={
              !desktop ||
              busy ||
              !text.trim() ||
              (!!coaching?.chat_bound && waiting)
            }
            aria-label="Send message"
          >
            <Send size={15} />
          </button>
        </div>
        {!coaching?.chat_bound && view.intent === "task" && (
          <button
            type="button"
            className="text-button"
            disabled={!desktop || busy || !text.trim() || text.length > 500}
            onClick={() =>
              void run(async () => {
                await api.companionSubmit(
                  text.trim(),
                  "task",
                  goal?.id ?? null,
                  plan?.revision ?? null,
                );
                setText("");
              })
            }
          >
            Add task locally
          </button>
        )}
        {!coaching?.chat_bound && view.intent === "selection" && (
          <div className="conversation-selection">
            <button
              type="button"
              className="text-button"
              disabled={!desktop || busy || !text.trim()}
              onClick={() => void submit("Explain this selected text: ")}
            >
              Explain
            </button>
            <button
              type="button"
              className="text-button"
              disabled={!desktop || busy || !text.trim() || text.length > 500}
              onClick={() =>
                void run(async () => {
                  await api.companionSubmit(
                    text.trim(),
                    "save",
                    goal?.id ?? null,
                    plan?.revision ?? null,
                  );
                  setText("");
                })
              }
            >
              Save for later
            </button>
          </div>
        )}
        {!compact && recording && (
          <p className="helper" role="status">
            Pauses are fine. Stop when you finish, then review before sending.
            Each recording can be up to 60 seconds.
          </p>
        )}
        {voicePreparation && (
          <p className="helper" role="status">
            {voicePreparation} Recordings stay on this device.
          </p>
        )}
        {compact && recording && (
          <p className="helper" role="status">
            Listening. Pauses are fine; press Stop when you finish.
          </p>
        )}
        {voiceProcessing && (
          <p className="helper" role="status">
            {voiceProcessing}
          </p>
        )}
        {voiceOriginal !== null && (
          <p className="helper" role="status">
            Voice text tidied.{" "}
            <button
              type="button"
              className="text-button"
              onClick={() => {
                setText(voiceOriginal);
                setVoiceOriginal(null);
              }}
            >
              Use original
            </button>
          </p>
        )}
        {voiceCleanupUnavailable && (
          <p className="helper" role="status">
            Couldn’t tidy the wording. Your original transcript is ready to
            edit.
          </p>
        )}
      </form>
      {compact && mode === "voice" && (
        <div
          className="compact-voice-bar"
          aria-label="Voice controls"
          title="Pauses are fine. Stop when you finish, then review before sending. Each recording can be up to 60 seconds."
        >
          <button
            type="button"
            aria-label="Switch to text"
            onClick={() => {
              cancelRecording();
              onMode?.("text");
            }}
          >
            <SquarePen size={17} />
          </button>
          <button
            type="button"
            disabled={!desktop || busy || !!voicePreparation}
            aria-label={
              recording
                ? muted
                  ? "Unmute microphone"
                  : "Mute microphone"
                : "Start recording"
            }
            onClick={() => {
              if (!recording) void startRecording();
              else {
                recorder.current?.setMuted(!muted);
                setMuted(!muted);
                onState(muted ? "listening" : "idle");
              }
            }}
          >
            {muted ? <MicOff size={18} /> : <Mic size={18} />}
          </button>
          <span
            className={`voice-indicator ${recording && !muted ? "listening" : ""}`}
            role="status"
            aria-label={
              busy
                ? "Transcribing"
                : recording
                  ? muted
                    ? "Microphone muted"
                    : "Listening"
                  : "Microphone ready"
            }
          />
          <button
            type="button"
            disabled={!recording || busy}
            aria-label="Stop and transcribe"
            onClick={() => void finishRecording()}
          >
            <Square size={16} />
          </button>
          <button
            type="button"
            aria-label="Close voice"
            onClick={() => {
              cancelRecording();
              onClose?.();
            }}
          >
            <X size={16} />
          </button>
        </div>
      )}
      {onResize && (
        <button
          className="chat-resize-handle text-button"
          aria-label="Resize sumi chat"
          onPointerDown={(event) => {
            event.preventDefault();
            onResize();
          }}
        >
          ↘
        </button>
      )}
    </section>
  );
}
