import { useEffect, useState } from "react";
import { api, desktop } from "../api/tauri";
import type { ServiceState } from "../ai/serviceStatus";

/** One provider as a row: name, status dot, and Test / Replace / Remove as small actions. */
export function ProviderRow({
  provider,
  name,
  role,
  configured,
  managed = false,
  checked,
  onChanged,
}: {
  provider: "anthropic";
  name: string;
  role: string;
  configured: boolean;
  managed?: boolean;
  /** Result of the last background check, when known. */
  checked?: ServiceState;
  onChanged: () => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{
    text: string;
    ok: boolean;
    detail?: string;
  } | null>(null);
  const [showDetail, setShowDetail] = useState(false);
  useEffect(() => {
    if (!note) return;
    if (!note.ok) return; // an error waits until the next action
    const timer = window.setTimeout(() => setNote(null), 4000);
    return () => window.clearTimeout(timer);
  }, [note]);
  async function act(work: () => Promise<unknown>, done: string) {
    setBusy(true);
    setNote(null);
    setShowDetail(false);
    try {
      await work();
      setNote({ text: done, ok: true });
    } catch (e) {
      const text = String(e).replace(/^Error:\s*/, "");
      // The short word stays in the row; the provider's full message is one click away.
      setNote({
        text: text.length > 40 ? "Failed" : text,
        ok: false,
        detail: text.length > 40 ? text : undefined,
      });
      setShowDetail(false);
    } finally {
      // Refresh the authoritative check after successes and failures alike.
      // A failed dashboard refresh must not turn a verified key into a failed key.
      try {
        await onChanged();
      } catch {
        // The parent owns refresh errors; the provider result above still stands.
      } finally {
        setBusy(false);
      }
    }
  }
  const off = !desktop || busy;
  // Saved is not the same as working: the dot follows the last real check.
  const verdict = !configured
    ? ["paused", "No key"]
    : checked === "verified"
      ? ["running", "Connected"]
      : checked === "unavailable"
        ? ["attention", "Could not verify access"]
        : checked === "expired"
          ? ["paused", "Check again"]
          : checked === "checking"
            ? ["paused", "Checking…"]
            : checked === "offline"
              ? ["paused", "Offline"]
              : [
                  "paused",
                  managed
                    ? "Built-in AI, not checked yet"
                    : "Key saved, not checked yet",
                ];
  const state = note ? (note.ok ? "running" : "attention") : verdict[0];
  return (
    <>
      <div className="setting-row provider-row">
        <span className="svc">
          <b>{name}</b>
          <small>{role}</small>
        </span>
        <span className="acts">
          <span className={`tracking-state ${state}`} role="status">
            <i aria-hidden="true" />
            {note?.detail ? (
              <button
                type="button"
                className="text-button row-link"
                title={note.detail}
                onClick={() => setShowDetail((open) => !open)}
              >
                {note.text} {showDetail ? "▴" : "▾"}
              </button>
            ) : note ? (
              note.text
            ) : (
              verdict[1]
            )}
          </span>
          {configured && (
            <button
              type="button"
              className="text-button row-link"
              disabled={off}
              onClick={() =>
                void act(() => api.testConnection(provider), "Connected")
              }
            >
              Test
            </button>
          )}
          <button
            type="button"
            className="text-button row-link"
            disabled={off || managed}
            title={
              managed
                ? "Choose Use my API keys to manage personal keys"
                : undefined
            }
            onClick={() => {
              setEditing((open) => !open);
              setKey("");
            }}
          >
            {editing ? "Cancel" : configured ? "Replace key" : "Add key"}
          </button>
          {configured && !editing && (
            <button
              type="button"
              className="text-button row-link muted"
              disabled={off || managed}
              title={
                managed
                  ? "Choose Use my API keys to manage personal keys"
                  : undefined
              }
              onClick={() =>
                void act(() => api.providerKey(provider, null), "Key removed")
              }
            >
              Remove
            </button>
          )}
        </span>
      </div>
      {showDetail && note?.detail && (
        <p className="helper provider-error" role="alert">
          {note.detail}
        </p>
      )}
      {editing && (
        <form
          className="setting-row provider-key-row"
          onSubmit={(event) => {
            event.preventDefault();
            if (!key.trim()) return;
            void act(
              () => api.providerKey(provider, key.trim()),
              "Key saved",
            ).then(() => {
              setEditing(false);
              setKey("");
            });
          }}
        >
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            maxLength={2048}
            aria-label={`${name} API key`}
            placeholder="Paste a new key"
            value={key}
            disabled={off}
            onChange={(event) => setKey(event.target.value)}
          />
          <button
            type="submit"
            className="small-button"
            disabled={off || !key.trim()}
          >
            Save
          </button>
        </form>
      )}
    </>
  );
}
