import type { CoreGoal, CoreSnapshot } from "../src/core/types";
import {
  defaultBuddyPreferences,
  defaultTrackingSettings,
  defaultUserSettings,
  type Dashboard,
} from "../src/types";
import { defaultCompanionView } from "../src/companion/types";
import { defaultMemoryControls } from "../src/api/memory";

export const NOW = "2026-10-20T10:00:00.000Z";
const date = "2026-10-20";
const goals: CoreGoal[] = [
  {
    id: 1,
    title: "Prepare the autumn release",
    area_id: 1,
    status: "open",
    focused_seconds: 2700,
    tracked_seconds: 3300,
    relevant_seconds: 2700,
    due_at: "2026-10-21T16:00:00Z",
    priority: "high",
    estimated_minutes: 90,
    created_at: "2026-10-19T09:00:00Z",
    description: "A small, thoughtful release.",
    plan: {
      goal_id: 1,
      revision: 1,
      done_when: "The release is ready to review",
      current_step: "review",
      steps: [
        { id: "notes", text: "Write the release notes", done: true },
        { id: "review", text: "Review the final changes", done: false },
      ],
    },
  },
  {
    id: 2,
    title: "Read the research notes",
    area_id: 2,
    status: "open",
    focused_seconds: 900,
    tracked_seconds: 1200,
    relevant_seconds: 900,
    due_at: null,
    priority: "medium",
    estimated_minutes: 30,
    created_at: "2026-10-19T10:00:00Z",
    plan: {
      goal_id: 2,
      revision: 1,
      done_when: "Three ideas are captured",
      current_step: null,
      steps: [
        { id: "read", text: "Summarise three useful ideas", done: false },
      ],
    },
  },
  {
    id: 3,
    title: "Plan a short walk",
    area_id: 2,
    status: "completed",
    focused_seconds: 600,
    tracked_seconds: 600,
    relevant_seconds: 600,
    completed_at: "2026-10-20T09:30:00Z",
    created_at: "2026-10-19T11:00:00Z",
    plan: {
      goal_id: 3,
      revision: 1,
      done_when: "A route is chosen",
      current_step: null,
      steps: [{ id: "route", text: "Choose a nearby route", done: true }],
    },
  },
];
const summary = {
  day: date,
  completed: ["Write the release notes"],
  completed_goals: ["Plan a short walk"],
  goals: goals.map((goal) => ({
    goal_id: goal.id,
    title: goal.title,
    area: goal.area_id === 1 ? "Work" : "Personal",
    seconds: goal.focused_seconds,
    tracked_seconds: goal.tracked_seconds,
    relevant_seconds: goal.relevant_seconds,
  })),
};
const core: CoreSnapshot = {
  date,
  selected_date: date,
  date_goals: [1, 2, 3],
  today: [1, 2, 3],
  carryover: [],
  day_mode: "plan",
  areas: [
    { id: 1, title: "Work" },
    { id: 2, title: "Personal" },
  ],
  goals,
  summary,
  week: Array.from({ length: 7 }, (_, index) => ({
    ...summary,
    day: `2026-10-${19 + index}`,
    goals: index === 1 ? summary.goals : [],
    completed: index === 1 ? summary.completed : [],
    completed_goals: index === 1 ? summary.completed_goals : [],
  })),
  hourly: [
    { hour: 9, goal_id: 1, seconds: 2700 },
    { hour: 10, goal_id: 2, seconds: 900 },
  ],
  timer: null,
  focused_goal_id: 1,
  preferences: {
    working_days: [1, 2, 3, 4, 5],
    movement_reminders: false,
    vision_model: "",
    ai_connection_mode: "built_in",
  },
};

function dashboard(theme: "light" | "dark", buddy: boolean): Dashboard {
  return {
    version: "0.39.0",
    app_rules: [],
    today: {
      date,
      nudges: 0,
      apps: [{ process_name: "Code", seconds: 3300, goal_seconds: 2700 }],
    },
    goal: { id: 1, text: goals[0].title, created_at: goals[0].created_at! },
    goal_plan: goals[0].plan,
    saved_goals: [],
    retention_days: 30,
    recommendations: [],
    activity: [],
    decision: null,
    status: {
      tracking: true,
      ai_enabled: false,
      dnd: false,
      demo: false,
      mock_ai: false,
      anthropic_configured: true,
      web_search_configured: true,
      last_collected_at: NOW,
    },
    user_settings: {
      ...defaultUserSettings,
      theme,
      profile: {
        ...defaultUserSettings.profile,
        name: "Alex",
        privacy_accepted: true,
      },
      onboarding: {
        ...defaultUserSettings.onboarding,
        completed: true,
        tracking_consent: true,
      },
    },
    buddy: {
      preferences: defaultBuddyPreferences,
      suggestion: null,
      decision: null,
      snoozed_until: null,
      quiet_reason: null,
      avatar: defaultUserSettings.profile.avatar,
      // This is an existing saved profile; fresh-install defaults must not change its choices.
      companion: {
        ...defaultCompanionView,
        preferences: {
          ...defaultCompanionView.preferences,
          use_screenshots: false,
          automatic_completion: false,
        },
        chat_open: buddy,
      },
    },
  };
}

// An explicit read-only allowlist. Unexpected IPC is a test failure, including
// provider requests, credentials, updates, and mutations.
export function fixtureReply(
  command: string,
  theme: "light" | "dark",
  buddy: boolean,
  review = false,
): unknown {
  switch (command) {
    case "get_dashboard":
      return dashboard(theme, buddy);
    case "get_core_snapshot":
      return core;
    case "get_core_drafts":
      return [];
    // Simulated read-only state; this never requests or grants OS access.
    case "get_accessibility_permission":
      return true;
    case "get_local_voice_languages":
      return ["en-GB"];
    case "get_tracking_settings":
      return defaultTrackingSettings;
    case "check_app_update":
      return {
        status: "current",
        current_version: "0.39.0",
        version: null,
        notes: null,
      };
    case "get_ai_service_status":
      return {
        connection_mode: "built_in",
        configured_model: null,
        configured_vision_model: "claude-haiku-5-5",
        configured_image_description_model: "claude-haiku-5-5",
        services: ["anthropic", "model", "search"].map((id) => ({
          id,
          status: "not_checked",
          checked_at: null,
        })),
      };
    case "get_ai_preferences":
      return {
        enabled: false,
        share_goal_context: false,
        web_research: false,
        automatic_goal_matching: false,
      };
    case "test_provider_connection":
      return "Connection not checked (visual fixture).";
    case "get_privacy_preview":
      return {
        focus_check: { recent_activity: [{ app: "Code", seconds: 3300 }] },
      };
    case "screenshot_permission":
      return false;
    case "get_unplanned_work_suggestion":
      return null;
    case "get_activity_review":
      if (review) return activityReviewFixture;
      return {
        pending_seconds: 0,
        pending_summary_count: 0,
        blocks: [],
        summaries: [],
      };
    case "visit_core_progress":
    case "set_activity_review_context":
      return null;
    case "get_goal_match_suggestion":
      return null;
    case "get_buddy_conversations":
      return [
        {
          id: 1,
          title: "A calm start to the day",
          updated_at: NOW,
          active: true,
        },
      ];
    case "get_buddy_chat_history":
      return [
        {
          id: 1,
          role: "user",
          text: "Help me choose a small next step.",
          created_at: "2026-10-20T09:00:00Z",
          goal_id: null,
          resources: [],
        },
        {
          id: 2,
          role: "assistant",
          text: "Start by reviewing one change. You can take the rest one step at a time.",
          created_at: "2026-10-20T09:01:00Z",
          goal_id: null,
          resources: [],
        },
      ];
    case "get_coaching_session":
      return null;
    case "get_memory_overview":
      return {
        controls: defaultMemoryControls,
        memories: [],
        contexts: [],
        goals: goals.map(({ id, title }) => ({ id, title })),
        proposals: [],
        more: false,
      };
    case "get_companion_view":
      return dashboard(theme, buddy).buddy.companion;
    case "get_avatar_color":
      return defaultUserSettings.profile.avatar.color;
    case "plugin:window|scale_factor":
      return 1;
    case "plugin:window|outer_position":
      return { x: 0, y: 0 };
    case "plugin:window|inner_position":
      return { x: 0, y: 0 };
    case "plugin:window|cursor_position":
      return { x: -40, y: -40 };
    case "plugin:window|is_visible":
      return true;
    case "plugin:window|set_ignore_cursor_events":
      return null;
    default:
      throw new Error(`Unmocked visual IPC: ${command}`);
  }
}

const reviewBlock = {
  key: "proposal:organize a community event",
  period_id: 7,
  day: date,
  started_at: "2026-10-20T09:00:00Z",
  ended_at: "2026-10-20T09:20:00Z",
  seconds: 720,
  apps: "Telegram, Browser",
  title: "Event planning",
  description: "Discussing the community event and researching possible venues",
  proposed_title: "Organize a community event",
  reason: "Related work outside the listed goals",
  status: "pending",
  goal_id: null,
  goal_title: null,
  evidence_ids: [],
  finalized: false,
};
const activityReviewFixture = {
  pending_seconds: 720,
  pending_summary_count: 1,
  blocks: [reviewBlock],
  summaries: [
    {
      period_id: 6,
      day: "2026-10-19",
      saved_at: "2026-10-19T18:02:00Z",
      results: ["Release notes reviewed (confirmed)"],
      blocks: [
        { ...reviewBlock, period_id: 6, day: "2026-10-19", finalized: true },
      ],
    },
  ],
};
