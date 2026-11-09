import { useEffect, useState } from "react";
import { Hint } from "../core/Hint";
import { desktop } from "../api/tauri";
import {
  memoryApi,
  type MemoryControls,
  type MemoryItem,
  type MemoryOverview,
  type WorkContext,
} from "../api/memory";

const memoryCategories = [
  "Personal knowledge",
  "Work",
  "Preferences",
  "Patterns",
  "Waiting",
  "Parked contexts",
] as const;
type Category = (typeof memoryCategories)[number];
function memoryCategory(item: MemoryItem): Category {
  if (
    item.memory_type === "explicit_fact" &&
    item.scope.scope_type === "global"
  )
    return "Personal knowledge";
  if (item.memory_type.includes("preference")) return "Preferences";
  if (item.memory_type === "behavioral_pattern") return "Patterns";
  if (item.event_kind === "waiting_state") return "Waiting";
  if (item.event_kind === "goal_parked") return "Parked contexts";
  return "Work";
}
/** Stored preference keys like "companion_preferences.screen_task_detection = true" read as words. */
function readable(content: string): string {
  const match = /^([a-z_]+)\.([a-z_]+) = (\S+)$/.exec(content);
  if (!match) return content;
  const [, , key, value] = match;
  const words = key.replaceAll("_", " ");
  const label = words.charAt(0).toUpperCase() + words.slice(1);
  const state =
    value === "true"
      ? "on"
      : value === "false"
        ? "off"
        : value.replaceAll("_", " ");
  return `${label} · ${state}`;
}
// Plain labels and one-sentence hints for the compact controls on the AI tab.
const COMPACT: Record<keyof MemoryControls, [string, string]> = {
  enabled: [
    "Remember",
    "Facts you tell sumi, where you stopped on each goal, and the ways of working you confirm. Kept on this computer.",
  ],
  remember_work_context: [
    "Where I left off",
    "Facts you ask it to keep, the step you stopped at, and what happened on each goal.",
  ],
  remember_preferences: [
    "How I like to work",
    "Choices you confirm in chat, and habits sumi notices once you approve them.",
  ],
  learn_from_feedback: [
    "Learn from my feedback",
    "Your thumbs up or down on suggestions shape the next ones. A habit counts only after it repeats on several days.",
  ],
  remember_conversation_context: [
    "Carry chats forward",
    "Uses what was said in a chat on the same goal next time, and may suggest a preference for you to approve.",
  ],
};
const toggles: [keyof MemoryControls, string, string][] = [
  [
    "enabled",
    "Memory",
    "Use and save personal memory. Turning this off keeps existing memories available to review and forget.",
  ],
  [
    "remember_work_context",
    "Remember work context",
    "Keep checkpoints, useful facts and meaningful goal events. No continuous screen text.",
  ],
  [
    "remember_preferences",
    "Remember preferences",
    "Keep explicit choices and preferences you confirm.",
  ],
  [
    "learn_from_feedback",
    "Learn from feedback",
    "Keep feedback events. Patterns need repeated evidence across multiple days.",
  ],
  [
    "remember_conversation_context",
    "Remember conversation context",
    "Use memories scoped to this chat and offer preference suggestions for your review. Chat history is separate.",
  ],
];
export function MemorySettings({
  onChanged = async () => {},
  initialData,
  part = "all",
}: {
  onChanged?: () => Promise<void>;
  initialData?: MemoryOverview;
  /** "controls" renders only the switches, "browser" only the saved memories and forget tools. */
  part?: "controls" | "browser" | "all";
}) {
  const [data, setData] = useState<MemoryOverview | null>(initialData ?? null);
  const [offset, setOffset] = useState(0);
  const [category, setCategory] = useState<Category>("Work");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [editing, setEditing] = useState<MemoryItem | WorkContext | null>(null);
  const [draft, setDraft] = useState("");
  const [knowledge, setKnowledge] = useState("");
  const [nextAction, setNextAction] = useState("");
  const [waitingFor, setWaitingFor] = useState("");
  const [confirmation, setConfirmation] = useState<{
    label: string;
    action: () => Promise<unknown>;
  } | null>(null);
  useEffect(() => {
    let active = true;
    if (desktop && !initialData)
      void memoryApi
        .overview()
        .then((value) => {
          if (active) setData(value);
        })
        .catch((e) => {
          if (active) setError(String(e));
        });
    return () => {
      active = false;
    };
  }, [initialData]);
  async function refresh() {
    setData(await memoryApi.overview());
    setOffset(0);
  }
  async function run(
    action: () => Promise<unknown>,
    success = "Memory updated.",
  ) {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      await action();
      await refresh();
      await onChanged();
      setMessage(success);
      setConfirmation(null);
      setEditing(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  function edit(item: MemoryItem | WorkContext) {
    setEditing(item);
    setConfirmation(null);
    setDraft("content" in item ? item.content : item.summary);
    setNextAction("next_action" in item ? (item.next_action ?? "") : "");
    setWaitingFor("waiting_for" in item ? (item.waiting_for ?? "") : "");
  }
  const blocked = !desktop || busy;
  return (
    <section aria-label="Personal memory" className="memory-settings">
      {part === "all" && (
        <p className="helper">
          Personal memory is separate from chat and activity history. It stays
          on this device. Relevant memories may be sent to Claude during chat
          only when AI goal-context sharing is enabled. Memory controls do not
          enable tracking or AI sharing. With web research enabled, generated
          web search queries may reflect the shared context.
        </p>
      )}
      {!data && (
        <p role="status">
          {desktop
            ? "Loading memory…"
            : "Memory is available in the desktop app."}
        </p>
      )}
      {data && (
        <>
          {part !== "browser" &&
            toggles.map(([key, label, help]) => (
              <label
                className={`toggle-row ${part === "controls" && key !== "enabled" ? "sub" : ""}`}
                key={key}
              >
                {part === "controls" ? (
                  <span className="lbl">
                    {COMPACT[key][0]}
                    <Hint text={COMPACT[key][1]} />
                  </span>
                ) : (
                  <span>
                    {label}
                    <small id={`memory-help-${key}`}>{help}</small>
                  </span>
                )}
                <input
                  type="checkbox"
                  aria-label={label}
                  aria-describedby={
                    part === "controls" ? undefined : `memory-help-${key}`
                  }
                  checked={data.controls[key]}
                  disabled={
                    blocked || (key !== "enabled" && !data.controls.enabled)
                  }
                  onChange={(e) =>
                    void run(
                      () =>
                        memoryApi.controls({
                          ...data.controls,
                          [key]: e.target.checked,
                        }),
                      "Memory controls saved.",
                    )
                  }
                />
              </label>
            ))}
          {part !== "controls" && (
            <>
              {part === "all" && <h3>View what sumi remembers</h3>}
              {category === "Personal knowledge" && (
                <div className="card">
                  <p>
                    Your main work, side projects and typical tasks. Add only
                    the context you want sumi to remember.
                  </p>
                  <textarea
                    aria-label="Personal knowledge"
                    value={knowledge}
                    maxLength={500}
                    disabled={
                      blocked ||
                      !data.controls.enabled ||
                      !data.controls.remember_work_context
                    }
                    onChange={(e) => setKnowledge(e.target.value)}
                  />
                  <button
                    disabled={
                      blocked ||
                      !knowledge.trim() ||
                      !data.controls.enabled ||
                      !data.controls.remember_work_context
                    }
                    onClick={() =>
                      void run(async () => {
                        await memoryApi.addKnowledge(knowledge);
                        setKnowledge("");
                      }, "Personal knowledge saved.")
                    }
                  >
                    Remember this
                  </button>
                </div>
              )}
              <div className="memory-filter" role="group" aria-label="Category">
                {memoryCategories.map((value) => (
                  <button
                    type="button"
                    key={value}
                    className="day-chip"
                    aria-pressed={category === value}
                    onClick={() => {
                      setCategory(value);
                      setEditing(null);
                      setConfirmation(null);
                    }}
                  >
                    {value}
                  </button>
                ))}
              </div>
              <ul className="memory-items">
                {data.memories
                  .filter((item) => memoryCategory(item) === category)
                  .map((item) => (
                    <li key={`memory-${item.id}`}>
                      <p>{readable(item.content)}</p>
                      <small>
                        {item.scope.scope_type === "goal"
                          ? (data.goals.find(
                              (g) => g.id === item.scope.scope_id,
                            )?.title ?? "Goal")
                          : item.scope.scope_type === "conversation"
                            ? `Chat ${item.scope.scope_id}`
                            : "Personal"}{" "}
                        · {item.memory_type.replaceAll("_", " ")}{" "}
                        {item.archived ? "· Archived" : ""}{" "}
                        {item.pinned ? "· Pinned" : ""}
                      </small>
                      {item.memory_type === "behavioral_pattern" ||
                      item.memory_type === "inferred_preference" ? (
                        <p className="helper">
                          Confidence: {Math.round(item.confidence * 100)}% ·{" "}
                          {item.evidence_count} supporting events
                        </p>
                      ) : null}
                      {item.archived &&
                        item.memory_type === "behavioral_pattern" && (
                          <button
                            disabled={
                              blocked ||
                              !data.controls.enabled ||
                              !data.controls.learn_from_feedback
                            }
                            onClick={() =>
                              void run(
                                () => memoryApi.confirmPattern(item.id),
                                "Work pattern confirmed.",
                              )
                            }
                          >
                            Use this pattern
                          </button>
                        )}
                      <div className="memory-actions">
                        <button
                          type="button"
                          disabled={blocked}
                          onClick={() => edit(item)}
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          disabled={blocked}
                          aria-pressed={item.pinned}
                          onClick={() =>
                            void run(() => memoryApi.pin(item.id, !item.pinned))
                          }
                        >
                          {item.pinned ? "Unpin" : "Pin"}
                        </button>
                        <button
                          type="button"
                          disabled={blocked}
                          onClick={() =>
                            setConfirmation({
                              label: "Forget this memory permanently?",
                              action: () => memoryApi.forget(item.id),
                            })
                          }
                        >
                          Forget
                        </button>
                      </div>
                    </li>
                  ))}
                {data.contexts
                  .filter(
                    (item) =>
                      (item.status === "waiting"
                        ? "Waiting"
                        : item.status === "paused"
                          ? "Parked contexts"
                          : "Work") === category,
                  )
                  .map((item) => (
                    <li key={`context-${item.id}`}>
                      <p>{item.summary}</p>
                      <small>
                        {data.goals.find((g) => g.id === item.goal_id)?.title ??
                          "Goal"}{" "}
                        · Checkpoint · {item.status}{" "}
                        {item.pinned ? "· Pinned" : ""}
                      </small>
                      {item.next_action && <p>Next: {item.next_action}</p>}
                      {item.waiting_for && (
                        <p>Waiting for: {item.waiting_for}</p>
                      )}
                      <div className="memory-actions">
                        <button
                          type="button"
                          disabled={
                            blocked ||
                            !data.controls.enabled ||
                            !data.controls.remember_work_context
                          }
                          onClick={() => edit(item)}
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          disabled={blocked}
                          aria-pressed={item.pinned}
                          onClick={() =>
                            void run(() =>
                              memoryApi.pinContext(item.goal_id, !item.pinned),
                            )
                          }
                        >
                          {item.pinned ? "Unpin" : "Pin"}
                        </button>
                        <button
                          type="button"
                          disabled={blocked}
                          onClick={() =>
                            setConfirmation({
                              label: "Forget this checkpoint permanently?",
                              action: () =>
                                memoryApi.forgetContext(item.goal_id),
                            })
                          }
                        >
                          Forget
                        </button>
                      </div>
                    </li>
                  ))}
              </ul>
              {!data.memories.some(
                (item) => memoryCategory(item) === category,
              ) &&
                !data.contexts.some(
                  (item) =>
                    (item.status === "waiting"
                      ? "Waiting"
                      : item.status === "paused"
                        ? "Parked contexts"
                        : "Work") === category,
                ) && <p>No memories in this category on the loaded pages.</p>}
              {data.more && (
                <button
                  type="button"
                  disabled={blocked}
                  onClick={async () => {
                    setBusy(true);
                    setError("");
                    try {
                      const page = await memoryApi.overview(offset + 50);
                      setData({
                        ...page,
                        memories: [...data.memories, ...page.memories],
                        contexts: [...data.contexts, ...page.contexts],
                      });
                      setOffset(offset + 50);
                    } catch (e) {
                      setError(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Load more memories
                </button>
              )}
              {editing && (
                <form
                  aria-label="Edit memory"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void run(() =>
                      "content" in editing
                        ? memoryApi.edit(editing, draft)
                        : memoryApi.editContext(
                            editing,
                            draft,
                            nextAction,
                            waitingFor,
                          ),
                    );
                  }}
                >
                  <label>
                    {"content" in editing
                      ? "Memory content"
                      : "Checkpoint summary"}
                    <textarea
                      required
                      maxLength={500}
                      value={draft}
                      disabled={blocked}
                      onChange={(e) => setDraft(e.target.value)}
                    />
                  </label>
                  {!("content" in editing) && (
                    <>
                      <label>
                        Next action
                        <input
                          maxLength={500}
                          value={nextAction}
                          disabled={blocked}
                          onChange={(e) => setNextAction(e.target.value)}
                        />
                      </label>
                      <label>
                        Waiting for
                        <input
                          required={editing.status === "waiting"}
                          maxLength={500}
                          value={waitingFor}
                          disabled={blocked}
                          onChange={(e) => setWaitingFor(e.target.value)}
                        />
                      </label>
                    </>
                  )}
                  <p className="helper">
                    Save confirms your correction. Editing an inferred pattern
                    makes it an explicit user fact or preference.
                  </p>
                  <button disabled={blocked || !draft.trim()}>
                    Save memory
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => setEditing(null)}
                  >
                    Cancel edit
                  </button>
                </form>
              )}
              {data.proposals.length > 0 && (
                <section aria-label="Memory suggestions">
                  <h3>Preferences to review</h3>
                  {part === "all" && (
                    <p>
                      These suggestions are not saved memories. Review the exact
                      text before accepting.
                    </p>
                  )}
                  {data.proposals.map((proposal) => (
                    <div key={proposal.id}>
                      <p>{proposal.content}</p>
                      <button
                        type="button"
                        disabled={
                          blocked ||
                          !data.controls.enabled ||
                          !data.controls.remember_preferences ||
                          !data.controls.remember_conversation_context
                        }
                        onClick={() =>
                          void run(() =>
                            memoryApi.resolveProposal(proposal.id, true),
                          )
                        }
                      >
                        Remember preference
                      </button>
                      <button
                        type="button"
                        disabled={blocked}
                        onClick={() =>
                          void run(
                            () => memoryApi.resolveProposal(proposal.id, false),
                            "Suggestion dismissed.",
                          )
                        }
                      >
                        Dismiss suggestion
                      </button>
                    </div>
                  ))}
                </section>
              )}
              {confirmation && (
                <div role="alertdialog" aria-label="Confirm memory deletion">
                  <p>{confirmation.label}</p>
                  <button
                    type="button"
                    disabled={blocked}
                    onClick={() =>
                      void run(confirmation.action, "Memory forgotten.")
                    }
                  >
                    Confirm deletion
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => setConfirmation(null)}
                  >
                    Cancel deletion
                  </button>
                </div>
              )}
            </>
          )}
        </>
      )}
      {error && <p role="alert">{error}</p>}
      {message && <p role="status">{message}</p>}
    </section>
  );
}
