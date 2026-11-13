import { useEffect, useState } from "react";
import { invoke } from "../data/ipc";
import { api, desktop } from "../api/tauri";
import type { CompanionPreferences } from "../companion/types";

export function ObservationSettings({
  preferences,
  onChanged,
}: {
  preferences: CompanionPreferences;
  onChanged: () => Promise<void>;
}) {
  const [permission, setPermission] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (desktop)
      void invoke<boolean>("screenshot_permission", { request: false })
        .then(setPermission)
        .catch(() => setPermission(false));
  }, []);
  async function update(patch: Partial<CompanionPreferences>) {
    setBusy(true);
    setError("");
    try {
      await api.companionPreferences({ ...preferences, ...patch });
      await onChanged();
    } catch {
      setError("Could not save observation settings. Try again.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <section
      className="settings-card settings-card-wide"
      aria-label="Observation permissions"
    >
      <p className="eyebrow settings-card-title">Understanding your work</p>
      <p>
        Uses the foreground app and available interface text across
        applications. Hidden tabs and conversations are unavailable. Quiet mode
        hides prompts; turn tracking off to stop observation.
      </p>
      <label className="toggle-row">
        <span>
          Use screenshots
          <small>
            When an app exposes little text, its active window is read on this
            device first; the image is not kept. Optional active-window images
            are sent to Claude to help understand layouts when text is
            insufficient. Images and interface-text samples
            are encrypted locally for review until the daily summary is saved at
            the configured Daily summary time, then deleted. Excluded and
            detected private windows are skipped. Sensitive content detection
            cannot be guaranteed.
          </small>
        </span>
        <input
          type="checkbox"
          checked={preferences.use_screenshots ?? false}
          disabled={!desktop || busy}
          onChange={(e) => void update({ use_screenshots: e.target.checked })}
        />
      </label>
      {preferences.use_screenshots && (
        <div className="setting-row">
          <span>
            System screen access:{" "}
            {permission === null
              ? "Checking"
              : permission
                ? "Allowed"
                : "Not granted"}
          </span>
          <button
            disabled={busy || !desktop}
            onClick={() =>
              void invoke<boolean>("screenshot_permission", { request: true })
                .then(setPermission)
                .catch(() => setPermission(false))
            }
          >
            Check permission
          </button>
        </div>
      )}
      <label className="toggle-row">
        <span>
          Automatically complete verified work
          <small>
            Allow completion of existing goals or steps only after a current
            result receipt passes two checks. Uncertain results and new tasks
            require your review. Undo remains available in Progress.
          </small>
        </span>
        <input
          type="checkbox"
          checked={preferences.automatic_completion ?? false}
          disabled={!desktop || busy}
          onChange={(e) =>
            void update({ automatic_completion: e.target.checked })
          }
        />
      </label>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
