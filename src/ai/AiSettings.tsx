import { useState } from "react";
import { desktop } from "../api/tauri";
import { aiApi, type AiPreferences } from "./api";
import { useQuery } from "../data/useQuery";
import { preferencesQuery } from "../data/resources";
import { queryClient } from "../data/client";
import { Hint } from "../core/Hint";
export function AiSettings({
  onChanged,
  compact = false,
  active = true,
}: {
  onChanged: () => Promise<void>;
  /** Plain switches with hints, no small print. */
  compact?: boolean;
  active?: boolean;
}) {
  const preferenceState = useQuery(preferencesQuery, desktop && active);
  const preferences = preferenceState.data;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function save(patch: Partial<AiPreferences>) {
    setBusy(true);
    setError("");
    try {
      const next = { ...preferences, ...patch };
      await aiApi.savePreferences(next);
      preferencesQuery.setData(next);
      await onChanged();
    } catch (e) {
      setError(queryClient.recordError("AI preferences", e));
    } finally {
      setBusy(false);
    }
  }
  const voiceCleanup = (
    <label className="toggle-row">
      <span className={compact ? "lbl" : undefined}>
        Correct dictated text before editing
        {compact ? (
          <Hint text="Before the draft appears, send the recognized text to Claude to fix likely misheard words. Includes up to 12 open goal titles only when goal sharing is on. Audio stays on your device. Nothing is sent as a chat message or changes your goals until Send. On for new profiles; you can switch it off here." />
        ) : (
          <small>
            Before the draft appears, send recognized text to Claude to fix
            likely misheard words. Includes up to 12 open goal titles only when
            goal sharing is on. Audio stays on your device. You review the
            result before Send.
          </small>
        )}
      </span>
      <input
        type="checkbox"
        checked={!!preferences.voice_text_cleanup}
        disabled={
          !desktop ||
          busy ||
          !preferenceState.ready ||
          !!preferenceState.error ||
          !preferences.enabled
        }
        onChange={(e) => void save({ voice_text_cleanup: e.target.checked })}
      />
    </label>
  );
  if (compact)
    return (
      <section aria-label="sumi AI sharing">
        <label className="toggle-row">
          <span className="lbl">
            Use AI
            <Hint text="Coach chat, goal suggestions and web search through the services above. Off stops every request." />
          </span>
          <input
            type="checkbox"
            checked={preferences.enabled}
            disabled={
              !desktop ||
              busy ||
              !preferenceState.ready ||
              !!preferenceState.error
            }
            onChange={(e) => void save({ enabled: e.target.checked })}
          />
        </label>
        <label className="toggle-row">
          <span className="lbl">
            Match my activity to goals
            <Hint text="Sends your open goals, foreground app, window/page titles and browser domain to Claude. With on-screen task detection enabled, also sends up to 400 characters of redacted visible text." />
          </span>
          <input
            type="checkbox"
            checked={preferences.automatic_goal_matching}
            disabled={
              !desktop ||
              busy ||
              !preferenceState.ready ||
              !!preferenceState.error
            }
            onChange={(e) =>
              void save({ automatic_goal_matching: e.target.checked })
            }
          />
        </label>
        {voiceCleanup}
        {(error || preferenceState.error) && (
          <p role="alert" className="error">
            {error || preferenceState.error}
          </p>
        )}
      </section>
    );
  return (
    <section aria-label="sumi AI sharing">
      <label className="toggle-row">
        <span>
          Allow sumi AI assistance
          <small>
            Automatically analyze new goals; send requests, conversation history
            and attachments you choose to Claude. Goal suggestions stay separate
            until you accept them. Provider charges may apply.
          </small>
        </span>
        <input
          type="checkbox"
          checked={preferences.enabled}
          disabled={
            !desktop ||
            busy ||
            !preferenceState.ready ||
            !!preferenceState.error
          }
          onChange={(e) => void save({ enabled: e.target.checked })}
        />
      </label>
      <label className="toggle-row">
        <span>
          Include Today goals in sumi chat
          <small>
            Share their titles, steps, deadlines and relevant activity totals
            and app names. Your profile, screen contents and window titles are
            excluded.
          </small>
        </span>
        <input
          type="checkbox"
          checked={preferences.share_goal_context}
          disabled={
            !desktop ||
            busy ||
            !preferenceState.ready ||
            !!preferenceState.error
          }
          onChange={(e) => void save({ share_goal_context: e.target.checked })}
        />
      </label>
      <label className="toggle-row">
        <span>
          Allow Tavily web research
          <small>
            sumi searches only when current information or an official workflow
            is useful. Tavily receives the search query; Claude receives
            returned source snippets.
          </small>
        </span>
        <input
          type="checkbox"
          checked={preferences.web_research}
          disabled={
            !desktop ||
            busy ||
            !preferenceState.ready ||
            !!preferenceState.error
          }
          onChange={(e) => void save({ web_research: e.target.checked })}
        />
      </label>
      <p className="helper">
        Automatic goal matching does not need a running timer.
      </p>
      <label className="toggle-row">
        <span>
          Let Claude identify the active goal
          <small>
            With tracking and AI assistance on, send up to 20 open goal titles
            and their next steps, plus the foreground app, available browser
            domain and window/page titles (up to 160 characters each), to Claude
            Token Factory. With on-screen task detection enabled, also send up
            to 400 characters of redacted visible text. A matching attempt
            starts at most once per minute; transient errors may be retried.
            Uncertain activity stays unassigned.
          </small>
        </span>
        <input
          type="checkbox"
          checked={preferences.automatic_goal_matching}
          disabled={
            !desktop ||
            busy ||
            !preferenceState.ready ||
            !!preferenceState.error
          }
          onChange={(e) =>
            void save({ automatic_goal_matching: e.target.checked })
          }
        />
      </label>
      {voiceCleanup}
      <p className="helper">
        AI assistance is {preferences.enabled ? "on" : "off"}. Activity tracking
        and automatic screen sampling have separate controls.
      </p>
      {(error || preferenceState.error) && (
        <p role="alert" className="error">
          {error || preferenceState.error}
        </p>
      )}
    </section>
  );
}
