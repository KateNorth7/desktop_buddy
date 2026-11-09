import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { defaultCompanionView } from "./companion/types";
import { api, desktop } from "./api/tauri";
import type { Dashboard } from "./types";
import { defaultBuddyPreferences, defaultUserSettings } from "./types";
import { DesktopBuddy } from "./components/DesktopBuddy";
import packageInfo from "../package.json";
import { useAvatarColorSync } from "./companion/useAvatarColorSync";
import { AppShell, pages, type WorkspacePage } from "./components/AppShell";
import { StartupUpdate, UpdateDialog } from "./components/Updates";
import { useUpdates } from "./updates/useUpdates";
import { ResetProfileDialog } from "./components/ResetProfileDialog";
import { Experience } from "./core/Experience";
import { CoreSettings } from "./core/Settings";
import { goalEnhancement } from "./ai/api";
import { queryClient } from "./data/client";
import { useQuery } from "./data/useQuery";
import { CoreOnboarding } from "./core/Onboarding";
import { WorkspaceOverview } from "./core/WorkspaceOverview";
import { FeedbackCard } from "./components/FeedbackCard";
import { useTheme } from "./useTheme";
const empty: Dashboard = {
  app_rules: [],
  today: { date: "", apps: [], nudges: 0 },
  goal_plan: null,
  saved_goals: [],
  retention_days: 0,
  recommendations: [],
  version: packageInfo.version,
  buddy: {
    preferences: defaultBuddyPreferences,
    suggestion: null,
    decision: null,
    quiet_reason: null,
  },
  goal: null,
  activity: [],
  decision: null,
  status: {
    tracking: false,
    ai_enabled: false,
    dnd: false,
    demo: false,
    mock_ai: false,
    anthropic_configured: false,
    tavily_configured: false,
  },
};
const dashboardQuery = queryClient.query(
  "dashboard",
  ["dashboard"],
  empty,
  () => api.dashboard(),
  15_000,
);
export default function App() {
  const dashboard = useQuery(dashboardQuery, desktop);
  const [previewData, setPreviewData] = useState<Dashboard>(empty);
  const data = desktop ? dashboard.data : previewData;
  const setData = useCallback(
    (next: Dashboard | ((previous: Dashboard) => Dashboard)) => {
      if (!desktop) {
        setPreviewData(next);
        return;
      }
      dashboardQuery.setData(next);
      // Appearance patches are partial; revalidate the rest of the dashboard too.
      queryClient.invalidate(["dashboard"]);
    },
    [],
  );
  useAvatarColorSync(setData);
  const [error, setError] = useState("");
  const displayedError = error || dashboard.error;
  const [busy, setBusy] = useState(false);
  const [draftDirty, setDraftDirty] = useState(false);
  const [settingsDirty, setSettingsDirty] = useState(false);
  const [setupDirty, setSetupDirty] = useState(false);
  const dirty = draftDirty || settingsDirty || setupDirty;
  const [page, setPage] = useState<WorkspacePage>("focus");
  const [settingsSection, setSettingsSection] = useState<{
    section: "Privacy" | "General";
    revision: number;
  }>();
  const loaded = !desktop || dashboard.ready;
  const [updatesOpen, setUpdatesOpen] = useState(false);
  const [resetOpen, setResetOpen] = useState(false);
  const [overview, setOverview] = useState(false);
  const [quickRequest, setQuickRequest] = useState(0);
  const [planningRequest, setPlanningRequest] = useState<{
    revision: number;
    tomorrow: boolean;
  }>();
  const closeOverview = useCallback(() => setOverview(false), []);
  const { theme, setTheme } = useTheme(data.user_settings);
  const popup = new URLSearchParams(location.search).has("buddy");
  const surfaceParam = new URLSearchParams(location.search).get("buddy");
  const surface =
    surfaceParam === "panel" || surfaceParam === "mascot"
      ? surfaceParam
      : "combined";
  const { controller, snapshot: update } = useUpdates(!popup);
  const startupBlocked = desktop && !popup && update.startup !== "ready";
  const settings = data.user_settings ?? defaultUserSettings;
  const preview =
    !desktop &&
    new URLSearchParams(location.search).get("preview") === "workspace";
  const previewProfileName = preview
    ? new URLSearchParams(location.search)
        .get("profileName")
        ?.trim()
        .slice(0, 120)
    : undefined;
  const onboarding = !settings.onboarding.completed && !preview;
  useEffect(() => {
    if (quickRequest && page === "focus" && !onboarding)
      document.querySelector<HTMLInputElement>(".quick-goal input")?.focus();
  }, [quickRequest, page, onboarding]);
  useEffect(() => {
    if (!desktop || popup) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<string>("buddy://navigate", (event) => {
      if (event.payload === "create-goal") {
        setPage("focus");
        setQuickRequest((previous) => previous + 1);
        return;
      }
      if (event.payload === "plan-today" || event.payload === "plan-tomorrow") {
        setPlanningRequest({
          revision: Date.now(),
          tomorrow: event.payload === "plan-tomorrow",
        });
        setPage("focus");
        return;
      }
      if (pages.some((item) => item.id === event.payload))
        setPage(event.payload as WorkspacePage);
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
    };
  }, [popup]);
  useEffect(() => {
    document.documentElement.classList.toggle("buddy-window", popup);
  }, [popup]);
  const refresh = useCallback(async () => {
    if (desktop) await dashboardQuery.refresh();
  }, []);
  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await action();
      await refresh();
      return true;
    } catch (e) {
      setError(queryClient.recordError("workspace action", e));
      return false;
    } finally {
      setBusy(false);
    }
  }
  if (popup)
    return (
      <main className="popup-shell">
        {displayedError && surface === "combined" && (
          <p role="alert">{displayedError}</p>
        )}
        <DesktopBuddy
          surface={surface}
          view={
            !desktop &&
            new URLSearchParams(location.search).get("preview") === "chat"
              ? {
                  ...data.buddy,
                  companion: { ...defaultCompanionView, chat_open: true },
                }
              : data.buddy
          }
          goal={data.goal}
          plan={data.goal_plan}
          onChanged={refresh}
          onDnd={() => void run(() => api.dnd(true))}
        />
      </main>
    );
  if (!loaded)
    return (
      <main className="onboarding-shell">
        <section className="card">
          <p role="status">Opening your workspace…</p>
          {displayedError && <p role="alert">{displayedError}</p>}
          <button onClick={() => void run(refresh)}>Retry</button>
        </section>
      </main>
    );
  const title = pages.find((item) => item.id === page)!;
  return (
    <>
      {startupBlocked && <StartupUpdate snapshot={update} />}
      <div inert={startupBlocked || updatesOpen || resetOpen}>
        <AppShell
          page={page}
          onNavigate={setPage}
          profile={
            previewProfileName
              ? { ...settings.profile, name: previewProfileName }
              : settings.profile
          }
          version={data.version}
          update={update}
          onUpdates={() => setUpdatesOpen(true)}
          onboarding={onboarding}
          theme={theme}
          quietMode={!!(data.status.dnd || data.buddy.companion?.quiet_mode)}
          quietScheduled={
            !data.status.dnd && !!data.buddy.companion?.quiet_mode
          }
          quietModeDisabled={busy || (!desktop && !preview)}
          onQuietMode={() => {
            if (!data.status.dnd && data.buddy.companion?.quiet_mode) {
              setSettingsSection({ section: "General", revision: Date.now() });
              setPage("settings");
              return;
            }
            if (desktop) void run(() => api.dnd(!data.status.dnd));
            else if (preview)
              setData((previous) => ({
                ...previous,
                status: { ...previous.status, dnd: !previous.status.dnd },
              }));
          }}
          onTheme={(next) => {
            if (desktop) void run(() => api.theme(next));
            else setTheme(next);
          }}
          onReset={() => setResetOpen(true)}
          onQuickGoal={() => {
            setPage("focus");
            setQuickRequest((previous) => previous + 1);
          }}
        >
          {onboarding ? (
            <CoreOnboarding
              initial={settings}
              onChanged={refresh}
              onDirty={setSetupDirty}
              onComplete={() => {
                setPage("focus");
                setOverview(false);
              }}
            />
          ) : (
            <>
              {overview && <WorkspaceOverview close={closeOverview} />}
              {page !== "chats" && (
                <div className="page-title">
                  <div>
                    <h1>{title.label}</h1>
                    {page !== "focus" && title.description && (
                      <p>{title.description}</p>
                    )}
                  </div>
                </div>
              )}
              {!desktop && (
                <p className="notice preview-notice">
                  Browser preview · saves, voice recognition and updates are
                  available in the Windows app.
                </p>
              )}
              {(data.status.demo || data.status.mock_ai) && (
                <p className="notice">
                  {data.status.demo ? "Simulated activity" : "Live activity"} ·{" "}
                  {data.status.mock_ai
                    ? "Mock AI responses — no Claude call"
                    : "Real Claude integration"}
                </p>
              )}
              {displayedError && (
                <p className="error" role="alert">
                  {displayedError}
                </p>
              )}
              {update.error && !startupBlocked && (
                <p className="notice" role="status">
                  Automatic update unavailable. Your installed version is ready
                  to use. Open Updates to retry.
                </p>
              )}
              <div
                hidden={
                  page === "settings" ||
                  page === "profile" ||
                  page === "feedback"
                }
              >
                <Experience
                  planningRequest={planningRequest}
                  dashboard={data}
                  onSettings={() => {
                    setSettingsSection({
                      section: "Privacy",
                      revision: Date.now(),
                    });
                    setPage("settings");
                  }}
                  enhancement={desktop ? goalEnhancement : undefined}
                  page={page}
                  onDirty={setDraftDirty}
                  onChanged={refresh}
                  revision={
                    String(data.goal?.id) +
                    ":" +
                    String(settings.onboarding.completed)
                  }
                />
              </div>
              {page === "feedback" && <FeedbackCard />}
              <div hidden={page !== "settings" && page !== "profile"}>
                <CoreSettings
                  data={data}
                  onChanged={refresh}
                  onDirty={setSettingsDirty}
                  onUpdates={() => setUpdatesOpen(true)}
                  sectionRequest={settingsSection}
                  active={page === "settings" || page === "profile"}
                />
              </div>
            </>
          )}
        </AppShell>
      </div>
      {updatesOpen && (
        <UpdateDialog
          controller={controller}
          snapshot={update}
          version={data.version}
          onClose={() => setUpdatesOpen(false)}
          unsaved={dirty}
        />
      )}{" "}
      {resetOpen && (
        <ResetProfileDialog
          busy={busy}
          unsaved={dirty}
          onClose={() => setResetOpen(false)}
          onConfirm={() =>
            void run(api.resetProfile).then((ok) => {
              if (ok) {
                setResetOpen(false);
                setDraftDirty(false);
                setSettingsDirty(false);
                setSetupDirty(false);
                setPage("focus");
              }
            })
          }
        />
      )}
    </>
  );
}
