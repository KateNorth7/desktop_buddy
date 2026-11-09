import { ObservationSettings } from "./ObservationSettings";
import { MemorySettings } from "../components/MemorySettings";
import { MacPermissions } from "../components/MacPermissions";
import { TrackingStatus } from "./TrackingStatus";
import { isMac } from "../api/platform";
import { invoke } from "../data/ipc";
import { useEffect, useState } from "react";
import { useAutosave } from "../components/useAutosave";
import { api, desktop } from "../api/tauri";
import { defaultUserSettings, type Dashboard } from "../types";
import { ProviderRow } from "./ProviderRow";
import { servicesQuery, workspaceQuery } from "../data/resources";
import { useQuery } from "../data/useQuery";
import { queryClient } from "../data/client";
import { openUrl } from "@tauri-apps/plugin-opener";
import { WindowActivity } from "./WindowActivity";
import { Hint } from "./Hint";
import { memoryApi } from "../api/memory";
import { AvatarPicker } from "../components/ProfileFields";
import { defaultCompanionPreferences } from "../companion/types";
import { CompanionSettings } from "../companion/CompanionSettings";
import { PersonalAiEnginePanel } from "../ai/PersonalAiEngine";
import { AiSettings } from "../ai/AiSettings";
import { coreApi } from "./api";
import { emptyCore } from "./types";

const sections = ["General", "Profile", "AI", "Privacy", "About"] as const;
export const PRIVACY_POLICY_URL =
  "https://artmarketvm.github.io/desktop_buddy/docs/privacy.html";

/** One plain sentence about what sumi knows right now and what it would send. */
export function describePrivacy(
  preview: string,
  tracking: boolean,
  goalTitle: string | null,
  screenshots = false,
): string {
  if (!tracking)
    return "Background tracking is paused. No new activity is collected; explicitly requested chat and research can still use the context you permit.";
  const sharing = `Enabled AI checks may send bounded interface text, window metadata, ${goalTitle ? "this goal" : "relevant goals"} and permitted relevant memories to Claude. ${screenshots ? "Optional active-window images may also be sent. Review evidence is encrypted locally until the daily summary is saved at the configured Daily summary time." : "Screenshots are off."} This preview does not show the exact next request.`;
  const goal = goalTitle ? `your goal, “${goalTitle}”` : "your current goal";
  let apps: { app: string; seconds: number }[];
  try {
    const parsed = JSON.parse(preview) as {
      focus_check?: {
        recent_activity?: { app: string; seconds: number }[];
      };
    };
    const totals = new Map<string, number>();
    for (const item of parsed.focus_check?.recent_activity ?? []) {
      // The lock screen is not something you "worked in".
      if (item.app === "loginwindow") continue;
      totals.set(item.app, (totals.get(item.app) ?? 0) + item.seconds);
    }
    apps = [...totals]
      .map(([app, seconds]) => ({ app, seconds }))
      .sort((x, y) => y.seconds - x.seconds)
      .slice(0, 3);
  } catch {
    apps = [];
  }
  if (!apps.length)
    return `sumi has not recorded any activity yet. Your permitted context may include ${goal}. ${sharing}`;
  const spell = ({ app, seconds }: { app: string; seconds: number }) => {
    const m = Math.round(seconds / 60);
    return `${app} for ${m < 1 ? "under a minute" : `${m} minute${m === 1 ? "" : "s"}`}`;
  };
  const list =
    apps.length === 1
      ? spell(apps[0])
      : `${apps
          .slice(0, -1)
          .map(spell)
          .join(", ")} and ${spell(apps[apps.length - 1])}`;
  return `Right now sumi knows you have been in ${list}. Your permitted context may include ${goal}. ${sharing}`;
}

export function CoreSettings({
  data,
  onChanged,
  onDirty,
  onUpdates,
  sectionRequest,
  active = true,
}: {
  data: Dashboard;
  onChanged: () => Promise<void>;
  onDirty: (dirty: boolean) => void;
  onUpdates: () => void;
  sectionRequest?: { section: "Privacy" | "General"; revision: number };
  active?: boolean;
}) {
  const settings = data.user_settings ?? defaultUserSettings;
  const [appearance, setAppearance] = useState(settings.profile.avatar);
  const identity = useAutosave(
    {
      name: settings.profile.name,
      email: settings.profile.email,
      autostart: settings.autostart,
    },
    (next) => coreApi.identity(next.name, next.autostart, null, next.email),
    onChanged,
    (next) =>
      !next.name.trim()
        ? "Enter your preferred name to save this change."
        : null,
  );
  const profile = {
    ...settings.profile,
    name: identity.value.name,
    email: identity.value.email,
    avatar: appearance,
  };
  const autostart = identity.value.autostart;
  const [goals, setGoals] = useState(emptyCore().goals);
  const [logGoal, setLogGoal] = useState<number | null>(null);
  const [forgetGoal, setForgetGoal] = useState<number | null>(null);
  const [forgetArmed, setForgetArmed] = useState(false);
  useEffect(() => {
    if (!forgetArmed) return;
    const timer = window.setTimeout(() => setForgetArmed(false), 5000);
    return () => window.clearTimeout(timer);
  }, [forgetArmed]);
  const [busy, setBusy] = useState(false);
  const [colorBusy, setColorBusy] = useState(false);
  const [ready, setReady] = useState(!desktop);
  const [error, setError] = useState("");

  const [trackingConsentPending, setTrackingConsentPending] = useState(false);
  const [retention, setRetention] = useState(data.retention_days);
  const [preview, setPreview] = useState("");
  const [panel, setPanel] = useState<"log" | "memory" | "raw" | null>(null);
  const [showDiagnostics, setShowDiagnostics] = useState(false);
  const [visionDraft, setVisionDraft] = useState<string | null>(null);
  const logLast = new Date().toISOString().slice(0, 10);
  const logFirst = new Date(Date.now() - 6 * 86_400_000)
    .toISOString()
    .slice(0, 10);
  const [clearPending, setClearPending] = useState(false);
  useEffect(() => {
    if (!clearPending) return;
    // An armed button disarms itself after five seconds.
    const timer = window.setTimeout(() => setClearPending(false), 5000);
    return () => window.clearTimeout(timer);
  }, [clearPending]);
  useEffect(() => setRetention(data.retention_days), [data.retention_days]);
  const trackingState = !data.status.tracking
    ? "paused"
    : data.status.tracking_error
      ? "attention"
      : "running";
  const trackingLabel =
    trackingState === "paused"
      ? "Paused"
      : trackingState === "attention"
        ? "Needs attention"
        : "Running";
  const [section, setSection] = useState<(typeof sections)[number]>("General");
  const serviceStateQuery = useQuery(
    servicesQuery,
    desktop && active && section === "AI",
  );
  const services = serviceStateQuery.error ? null : serviceStateQuery.data;
  const providerChanged = async () => {
    await servicesQuery.refresh();
    await onChanged();
  };
  const serviceState = (id: "anthropic") =>
    services?.services.find((item) => item.id === id)?.status;
  useEffect(() => {
    if (section !== "Privacy" || !desktop) return;
    let active = true;
    void api
      .privacyPreview()
      .then((snapshot) => {
        if (active) setPreview(JSON.stringify(snapshot, null, 2));
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [section, data.status.last_collected_at]);
  const privacySentence = describePrivacy(
    preview,
    data.status.tracking,
    data.goal?.text ?? null,
    data.buddy.companion?.preferences.use_screenshots ?? false,
  );
  useEffect(() => {
    if (sectionRequest) setSection(sectionRequest.section);
  }, [sectionRequest]);
  const workspace = useQuery(workspaceQuery(null, "week"), desktop && active);
  const preferenceSave = useAutosave(
    workspace.data.preferences,
    (next) => coreApi.preferences(next),
    async () => {
      await workspaceQuery(null, "week").refresh();
      await onChanged();
    },
  );
  const preferences = preferenceSave.value;
  const dirty = identity.pending || preferenceSave.pending;
  useEffect(() => {
    if (!workspace.ready) return;
    setGoals(workspace.data.goals);
    setReady(true);
  }, [workspace.data.goals, workspace.ready]);
  useEffect(() => {
    onDirty(dirty);
    return () => onDirty(false);
  }, [dirty, onDirty]);
  const blocked = busy || colorBusy || !desktop || !ready;
  async function selectColor(next: typeof profile) {
    const previous = profile.avatar.color;
    setAppearance(next.avatar);
    setColorBusy(true);
    setError("");
    let committed = false;
    try {
      await api.setAvatarColor(next.avatar.color);
      committed = true;
      await onChanged();
    } catch (e) {
      if (!committed)
        setAppearance((current) => ({ ...current, color: previous }));
      setError(queryClient.recordError("settings action", e));
    } finally {
      setColorBusy(false);
    }
  }
  async function run(work: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await work();
      await onChanged();
      return true;
    } catch (e) {
      setError(queryClient.recordError("settings action", e));
      return false;
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="core-workspace core-settings">
      <nav className="settings-sections" aria-label="Settings sections">
        {sections.map((item) => (
          <button
            key={item}
            type="button"
            className={section === item ? "selected" : ""}
            aria-current={section === item ? "page" : undefined}
            onClick={() => setSection(item)}
          >
            {item}
          </button>
        ))}
      </nav>
      <div className="settings-drafts">
        <details className="core-section" hidden={section !== "Profile"} open>
          <summary>Profile</summary>
          <fieldset disabled={blocked}>
            <label>
              Preferred name
              <input
                required
                maxLength={120}
                value={profile.name}
                onBlur={() => void identity.flush()}
                onChange={(e) => {
                  identity.update(
                    { ...identity.value, name: e.target.value },
                    350,
                  );
                }}
              />
            </label>
            <label>
              Email (optional)
              <input
                type="email"
                maxLength={254}
                value={profile.email}
                onBlur={() => void identity.flush()}
                onChange={(e) => {
                  identity.update(
                    { ...identity.value, email: e.target.value },
                    350,
                  );
                }}
              />
            </label>
          </fieldset>
        </details>
        <details className="core-section" hidden={section !== "General"} open>
          <summary>General</summary>
          <AvatarPicker
            profile={profile}
            disabled={blocked}
            onChange={(next) => void selectColor(next)}
          />
          <div className="settings-cards">
            <section className="settings-card" aria-label="On your desktop">
              <p className="eyebrow settings-card-title">On your desktop</p>
              <label className="toggle-row">
                <span>Show sumi on desktop</span>
                <input
                  type="checkbox"
                  checked={
                    settings.profile.avatar.visible &&
                    !data.buddy.preferences.suggestions_only
                  }
                  disabled={blocked}
                  onChange={(e) => {
                    const visible = e.target.checked;
                    void run(() => api.showBuddy(visible)).then((ok) => {
                      if (ok)
                        setAppearance((previous) => ({ ...previous, visible }));
                    });
                  }}
                />
              </label>
              {!data.buddy.desktop_visible && data.buddy.visibility_reason && (
                <p className="helper" role="status">
                  {data.buddy.visibility_reason}
                </p>
              )}
              <label className="toggle-row">
                <span>Start when I sign in</span>
                <input
                  type="checkbox"
                  checked={autostart}
                  disabled={blocked}
                  onChange={(e) => {
                    identity.update({
                      ...identity.value,
                      autostart: e.target.checked,
                    });
                  }}
                />
              </label>
              <CompanionSettings
                part="desktop"
                onChanged={onChanged}
                currentPreferences={data.buddy.companion?.preferences}
              />
              <label className="toggle-row">
                <span>Use system appearance</span>
                <input
                  type="checkbox"
                  checked={settings.follow_system_theme ?? true}
                  disabled={blocked}
                  onChange={(event) => {
                    const theme = event.target.checked
                      ? "system"
                      : document.documentElement.dataset.theme === "dark"
                        ? "dark"
                        : "light";
                    void run(() => api.theme(theme));
                  }}
                />
              </label>
              <p className="helper">
                Automatically switch between light and dark with your computer.
                The sidebar theme button switches to a manual choice.
              </p>
              <label className="toggle-row">
                <span>Remind me to stretch</span>
                <input
                  type="checkbox"
                  checked={preferences.movement_reminders}
                  disabled={blocked}
                  onChange={(e) => {
                    const enabled = e.target.checked;
                    preferenceSave.update({
                      ...preferences,
                      movement_reminders: enabled,
                    });
                  }}
                />
              </label>
              <label className="setting-row">
                <span>Stretch interval</span>
                <select
                  aria-label="Stretch interval"
                  value={preferences.movement_interval_minutes ?? 45}
                  disabled={blocked || !preferences.movement_reminders}
                  onChange={(event) => {
                    const next = {
                      ...preferences,
                      movement_interval_minutes: Number(event.target.value),
                    };
                    preferenceSave.update(next);
                  }}
                >
                  {[15, 30, 45, 60, 90, 120].map((minutes) => (
                    <option key={minutes} value={minutes}>
                      {minutes} minutes
                    </option>
                  ))}
                </select>
              </label>
            </section>
            <section className="settings-card" aria-label="Nudges">
              <p className="eyebrow settings-card-title">Nudges</p>
              <CompanionSettings
                part="nudges"
                onChanged={onChanged}
                currentPreferences={data.buddy.companion?.preferences}
              />
            </section>
          </div>
        </details>
        <details className="core-section" hidden={section !== "Privacy"} open>
          <summary>Privacy</summary>
          <section
            className="settings-card settings-card-wide"
            aria-label="Tracking"
          >
            <p className="eyebrow settings-card-title">Tracking</p>
            <label className="toggle-row">
              <span>Track goals and suggest completed work</span>
              <input
                type="checkbox"
                checked={data.status.tracking}
                disabled={blocked}
                onChange={(e) => {
                  const enabled = e.target.checked;
                  if (enabled) {
                    setTrackingConsentPending(true);
                    return;
                  }
                  void run(() => api.goalMonitoring(enabled));
                }}
              />
            </label>
            {trackingConsentPending && (
              <div
                className="card"
                aria-label="Confirm local activity tracking"
              >
                <p>
                  Record foreground apps, window/tab titles and active time on
                  this computer, and send up to 3,000 characters of visible text
                  with your goal and steps to Claude for progress checks?
                  Automatic goal matching, when enabled, can also use up to 400
                  characters of the same redacted text to identify your goal.
                  Password and edit controls are skipped. Provider charges may
                  apply.
                </p>
                <button
                  disabled={blocked}
                  onClick={() => {
                    void run(() => api.goalMonitoring(true)).then(
                      async (ok) => {
                        if (ok) setTrackingConsentPending(false);
                        if (ok && isMac)
                          await invoke<boolean>(
                            "request_accessibility_permission",
                          ).catch((e) => setError(String(e)));
                      },
                    );
                  }}
                >
                  Enable goal tracking
                </button>
                <button
                  className="text-button"
                  disabled={blocked}
                  onClick={() => setTrackingConsentPending(false)}
                >
                  Cancel
                </button>
              </div>
            )}
            <div className="setting-row">
              <span>Status</span>
              <span className={`tracking-state ${trackingState}`}>
                <i aria-hidden="true" />
                {trackingLabel}
              </span>
            </div>
            <MacPermissions variant="row" />

            <label className="setting-row">
              <span>Keep history</span>
              <select
                value={retention}
                disabled={blocked}
                onChange={(e) => {
                  const days = Number(e.target.value);
                  setRetention(days);
                  void run(() => api.retention(days));
                }}
              >
                <option value={0}>Until I delete it</option>
                <option value={7}>7 days</option>
                <option value={30}>30 days</option>
                <option value={90}>90 days</option>
              </select>
            </label>
            <div className="setting-row">
              <button
                type="button"
                className={`clear-history ${clearPending ? "armed" : ""}`}
                disabled={blocked}
                onClick={() => {
                  if (!clearPending) {
                    setClearPending(true);
                    return;
                  }
                  void run(async () => {
                    await api.clearHistory();
                    setPreview("");
                  }).then(() => setClearPending(false));
                }}
              >
                {clearPending
                  ? "Click again to clear everything"
                  : "Clear all local history"}
              </button>
            </div>
          </section>
          <ObservationSettings
            preferences={
              data.buddy.companion?.preferences ?? defaultCompanionPreferences
            }
            onChanged={onChanged}
          />
          <div className="privacy-sentence">
            <span className="privacy-cell" aria-hidden="true" />
            <p role="status">{privacySentence}</p>
          </div>
          <div className="privacy-actions">
            <button
              type="button"
              className={`privacy-action ${panel === "log" ? "on" : ""}`}
              aria-pressed={panel === "log"}
              onClick={() => setPanel(panel === "log" ? null : "log")}
            >
              See the activity log
            </button>
            <button
              type="button"
              className={`privacy-action ${panel === "memory" ? "on" : ""}`}
              aria-pressed={panel === "memory"}
              onClick={() => setPanel(panel === "memory" ? null : "memory")}
            >
              See what sumi remembers
            </button>
            <button
              type="button"
              className={`privacy-action ${panel === "raw" ? "on" : ""}`}
              aria-pressed={panel === "raw"}
              disabled={blocked}
              onClick={() => setPanel(panel === "raw" ? null : "raw")}
            >
              Raw data
            </button>
            <button
              type="button"
              className="privacy-action"
              onClick={() => void openUrl(PRIVACY_POLICY_URL)}
            >
              Privacy policy ↗
            </button>
          </div>
          {panel === "log" && (
            <section
              className="settings-card activity-log"
              aria-label="Activity log"
            >
              <div className="activity-log-head">
                <p className="eyebrow settings-card-title">
                  Activity log · last 7 days
                </p>
                <label className="activity-log-goal">
                  <span className="sr-only">Goal</span>
                  <select
                    value={logGoal ?? ""}
                    onChange={(e) =>
                      setLogGoal(e.target.value ? Number(e.target.value) : null)
                    }
                  >
                    <option value="">All goals</option>
                    {goals
                      .filter((goal) => goal.status !== "completed")
                      .map((goal) => (
                        <option key={goal.id} value={goal.id}>
                          {goal.title}
                        </option>
                      ))}
                  </select>
                </label>
              </div>
              <WindowActivity
                first={logFirst}
                last={logLast}
                goalId={logGoal}
              />
            </section>
          )}
          {panel === "memory" && (
            <section
              className="settings-card activity-log"
              aria-label="What sumi remembers"
            >
              <p className="eyebrow settings-card-title">What sumi remembers</p>
              <MemorySettings part="browser" onChanged={onChanged} />
            </section>
          )}
          {panel === "raw" && preview && (
            <pre className="privacy-preview">{preview}</pre>
          )}
        </details>
        <details className="core-section" hidden={section !== "AI"} open>
          <summary>AI</summary>
          <div className="settings-cards">
            <section
              className="settings-card settings-card-full"
              aria-label="Services"
            >
              <p className="eyebrow settings-card-title">Services</p>
              <label className="setting-row">
                <span className="svc">AI connection</span>
                <select
                  aria-label="AI connection"
                  value={preferences.ai_connection_mode ?? "built_in"}
                  disabled={
                    blocked || !workspace.ready || preferenceSave.pending
                  }
                  onChange={(event) =>
                    preferenceSave.update({
                      ...preferences,
                      ai_connection_mode: event.target.value as
                        "built_in" | "personal",
                    })
                  }
                >
                  <option value="built_in">Built-in AI</option>
                  <option value="personal">Use my API keys</option>
                </select>
              </label>
              <ProviderRow
                key={`anthropic:${preferences.ai_connection_mode ?? "built_in"}`}
                provider="anthropic"
                name="Claude"
                managed={
                  (preferences.ai_connection_mode ?? "built_in") === "built_in"
                }
                role={
                  services?.configured_model
                    ? `runs the AI · ${services.configured_model}`
                    : "runs the AI"
                }
                configured={data.status.anthropic_configured}
                checked={serviceState("anthropic")}
                onChanged={providerChanged}
              />
              <div
                className="setting-row provider-row vision-model-row"
                role="group"
                aria-label="Vision model"
              >
                <span className="svc">
                  <b>Vision model</b>
                  <small>
                    screenshots ·{" "}
                    {services?.configured_vision_model ||
                      (serviceStateQuery.loading
                        ? "Loading…"
                        : "Model unavailable")}
                  </small>
                  <Hint
                    text={`Shares the Claude key above. Screen understanding: ${services?.configured_vision_model || "unavailable"}. Image descriptions: ${services?.configured_image_description_model || "unavailable"}.`}
                  />
                </span>
                <span className="acts">
                  <span className="tracking-state paused">
                    <i aria-hidden="true" />
                    Uses Claude
                  </span>
                  <button
                    type="button"
                    className="text-button row-link"
                    disabled={blocked}
                    onClick={() =>
                      setVisionDraft(
                        visionDraft === null
                          ? preferences.vision_model ||
                              services?.configured_vision_model ||
                              ""
                          : null,
                      )
                    }
                  >
                    {visionDraft === null ? "Change model" : "Cancel"}
                  </button>
                  {preferences.vision_model && visionDraft === null && (
                    <button
                      type="button"
                      className="text-button row-link muted"
                      disabled={blocked}
                      onClick={() =>
                        preferenceSave.update({
                          ...preferences,
                          vision_model: "",
                        })
                      }
                    >
                      Use default
                    </button>
                  )}
                </span>
              </div>
              {visionDraft !== null && (
                <form
                  className="setting-row provider-key-row vision-model-editor"
                  onSubmit={(event) => {
                    event.preventDefault();
                    preferenceSave.update({
                      ...preferences,
                      vision_model: visionDraft.trim(),
                    });
                    setVisionDraft(null);
                  }}
                >
                  <input
                    type="text"
                    aria-label="Vision model ID"
                    maxLength={160}
                    autoComplete="off"
                    spellCheck={false}
                    value={visionDraft}
                    disabled={blocked}
                    placeholder="Model ID (empty uses default)"
                    onChange={(event) => setVisionDraft(event.target.value)}
                  />
                  <button
                    type="submit"
                    className="small-button"
                    disabled={blocked}
                  >
                    Save model
                  </button>
                </form>
              )}
              <p className="helper">
                Connection tests verify key access. AI generation and web search
                can still fail or time out separately.
              </p>
            </section>
            <section className="settings-card" aria-label="AI">
              <p className="eyebrow settings-card-title">AI</p>
              <AiSettings
                onChanged={onChanged}
                compact
                active={active && section === "AI"}
              />
            </section>
            <section className="settings-card" aria-label="Memory">
              <p className="eyebrow settings-card-title">Memory</p>
              <MemorySettings part="controls" onChanged={onChanged} />
              <div className="setting-row forget-goal-row">
                <span className="lbl">
                  Forget a goal
                  <Hint text="Removes everything sumi remembers about one goal: facts, checkpoints and events. The goal itself stays." />
                </span>
                <span className="acts">
                  <select
                    aria-label="Goal to forget"
                    value={forgetGoal ?? ""}
                    disabled={blocked}
                    onChange={(e) => {
                      setForgetGoal(
                        e.target.value ? Number(e.target.value) : null,
                      );
                      setForgetArmed(false);
                    }}
                  >
                    <option value="">Choose a goal</option>
                    {goals.map((goal) => (
                      <option key={goal.id} value={goal.id}>
                        {goal.title}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className={`small-button ${forgetArmed ? "danger-button" : "outline-button"}`}
                    disabled={blocked || forgetGoal === null}
                    onClick={() => {
                      if (forgetGoal === null) return;
                      if (!forgetArmed) {
                        setForgetArmed(true);
                        return;
                      }
                      const id = forgetGoal;
                      setForgetArmed(false);
                      void run(() => memoryApi.clearGoal(id)).then((ok) => {
                        if (ok) setForgetGoal(null);
                      });
                    }}
                  >
                    {forgetArmed ? "Click again to forget" : "Forget"}
                  </button>
                </span>
              </div>
              <div className="setting-row">
                <span />
                <button
                  type="button"
                  className="text-button row-link"
                  onClick={() => {
                    setPanel("memory");
                    setSection("Privacy");
                  }}
                >
                  See what sumi remembers ↗
                </button>
              </div>
            </section>
          </div>
        </details>
        <details className="core-section" hidden={section !== "About"} open>
          <summary>About</summary>
          <p>sumi · v{data.version}</p>
          <div className="about-actions">
            <button
              className="outline-button"
              type="button"
              onClick={onUpdates}
            >
              Check for updates
            </button>
            <button
              className="text-button"
              type="button"
              onClick={() => setShowDiagnostics((open) => !open)}
            >
              {showDiagnostics ? "Hide diagnostics" : "Diagnostics"}
            </button>
          </div>
          {showDiagnostics && (
            <div className="diagnostics">
              <TrackingStatus dashboard={data} compact />
              <PersonalAiEnginePanel
                initiallyOpen
                active={active && section === "About"}
              />
            </div>
          )}
        </details>
        <div
          className="settings-actions settings-save"
          role="status"
          aria-live="polite"
        >
          {identity.error || preferenceSave.error ? (
            <>
              <span className="error">
                {identity.error || preferenceSave.error}
              </span>
              <button
                type="button"
                className="text-button"
                onClick={() => {
                  if (identity.error) void identity.flush();
                  if (preferenceSave.error) void preferenceSave.flush();
                }}
              >
                Retry
              </button>
            </>
          ) : (
            <span className="helper">
              {dirty
                ? "Saving changes…"
                : identity.status === "saved" ||
                    preferenceSave.status === "saved"
                  ? "All changes saved"
                  : "Changes save automatically"}
            </span>
          )}
        </div>
      </div>

      {(error ||
        workspace.error ||
        (section === "AI" && serviceStateQuery.error)) && (
        <p className="error" role="alert">
          {error || workspace.error || serviceStateQuery.error}
        </p>
      )}
    </div>
  );
}
