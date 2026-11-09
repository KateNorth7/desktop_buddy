import type { CompanionView } from "./companion/types";
export interface Goal {
  id: number;
  text: string;
  created_at: string;
}
export interface GoalStep {
  id: string;
  text: string;
  done: boolean;
}
export interface GoalPlan {
  goal_id: number;
  revision: number;
  done_when: string;
  steps: GoalStep[];
  current_step: string | null;
}
interface SavedGoal {
  id: number;
  text: string;
  status: "deferred" | "completed";
}
interface ActivitySnapshot {
  window_id?: number | null;
  browser?: {
    browser: string;
    page_title: string;
    domain: string | null;
    source: string;
  } | null;
  media_playing?: boolean;
  timestamp: string;
  process_name: string;
  window_title: string;
  idle_seconds: number;
  active_seconds: number;
}
export interface Decision {
  id?: number;
  state: "focused" | "drifting" | "stuck";
  confidence: number;
  reason: string;
  action: "intervene" | "wait" | "offer_help";
}
export interface SearchResult {
  title: string;
  url: string;
  content: string;
}
interface Status {
  tracking: boolean;
  tracking_error?: string | null;
  last_collected_at?: string | null;
  ai_enabled: boolean;
  dnd: boolean;
  demo: boolean;
  mock_ai: boolean;
  anthropic_configured: boolean;
  tavily_configured: boolean;
}
interface PersonalWorkState {
  active_goal_id: number | null;
  active_step_id: string | null;
  state: "working" | "waiting" | "stuck" | "drifting" | "away";
  confidence: number;
  last_meaningful_event: string | null;
  current_work_context: Record<string, unknown> | null;
  waiting_for: string | null;
  relevant_evidence: string[];
}
export interface Dashboard {
  work_state?: PersonalWorkState;
  goal_matching?: {
    source?: string | null;
    goal_id: number | null;
    confidence: number | null;
    reason: string | null;
    enabled: boolean;
  };
  user_settings?: UserSettings;
  app_rules: AppRule[];
  today: Today;
  goal_plan: GoalPlan | null;
  saved_goals: SavedGoal[];
  retention_days: number;
  recommendations: Recommendation[];
  version: string;
  buddy: BuddyView;
  goal: Goal | null;
  activity: ActivitySnapshot[];
  decision: Decision | null;
  status: Status;
  last_error?: string | null;
}
export interface BuddyPreferences {
  local_nudges: boolean;
  daily_nudge_limit: number;
  nudge_interval_minutes: number;
  suggestions_only: boolean;
  proactive: boolean;
  interval_minutes: number;
  suppress_fullscreen: boolean;
  suppress_meetings: boolean;
  wait_for_input_pause: boolean;
  excluded_apps: string[];
}
export const defaultBuddyPreferences: BuddyPreferences = {
  local_nudges: false,
  daily_nudge_limit: 8,
  nudge_interval_minutes: 15,
  suggestions_only: false,
  proactive: false,
  interval_minutes: 15,
  suppress_fullscreen: true,
  suppress_meetings: true,
  wait_for_input_pause: true,
  excluded_apps: [],
};
export interface BuddyView {
  panel?: {
    generation: number;
    arrow_offset?: number;
    mode: number;
    side: "above" | "below" | "left" | "right";
  } | null;
  mascot_offset?: [number, number];
  companion?: CompanionView;
  avatar?: AvatarPreferences;
  activity_state?: "focused" | "paused" | "drifting";
  activity_event?:
    "working" | "paused" | "drifting" | "resumed" | "goal_completed";
  activity_revision?: number;
  completion_revision?: number;
  background_thinking?: boolean;
  preferences: BuddyPreferences;
  suggestion: { id: number; title: string; url: string; reason: string } | null;
  decision: Decision | null;
  quiet_reason: string | null;
  desktop_visible?: boolean;
  visibility_reason?: string | null;
}
interface Recommendation {
  id: number;
  goal_id: number;
  goal: string;
  title: string;
  url: string;
  reason: string;
  created_at: string;
  feedback: boolean | null;
}
type AppCategory = "work" | "distraction" | "neutral";
interface AppRule {
  preset_source?: string | null;
  process_name: string;
  category: AppCategory;
}
interface Today {
  date: string;
  apps: { process_name: string; seconds: number; goal_seconds: number }[];
  nudges: number;
}

export interface TrackingSettings {
  idle_seconds: number;
  drifting_context_seconds: number;
  drifting_no_input_seconds: number;
  working_start_minute: number;
  working_end_minute: number;
  browser_metadata: boolean;
}
export const defaultTrackingSettings: TrackingSettings = {
  idle_seconds: 300,
  drifting_context_seconds: 180,
  drifting_no_input_seconds: 120,
  working_start_minute: 540,
  working_end_minute: 1080,
  browser_metadata: true,
};
interface ContextTime {
  process_name: string;
  domain: string | null;
  page_title: string | null;
  active_milliseconds: number;
  idle_milliseconds: number;
  drifting_milliseconds: number;
}
export interface GoalHistory {
  goal: Goal;
  status: string;
  completed_at: string | null;
  elapsed_milliseconds: number | null;
  first_observed_at: string | null;
  last_observed_at: string | null;
  active_milliseconds: number;
  unattributed_active_milliseconds: number;
  focused_milliseconds: number;
  work_app_milliseconds: number;
  idle_milliseconds: number;
  drifting_milliseconds: number;
  distraction_app_milliseconds: number;
  drifting_events: number;
  pause_events: number;
  interventions: number;
  applications: ContextTime[];
  sites: ContextTime[];
  tabs: ContextTime[];
  recommendations: Recommendation[];
}

type Character = "cat" | "dog" | "seal" | "bird";
export type AvatarState =
  | "working"
  | "paused"
  | "sleeping"
  | "completed"
  | "fun"
  | "idle"
  | "listening"
  | "thinking"
  | "success"
  | "stretching"
  | "attention";
export interface AvatarPreferences {
  character: Character;
  color: string;
  visible: boolean;
}
export interface UserProfile {
  name: string;
  email: string;
  role: string;
  custom_role?: string;
  applications?: string[];
  privacy_accepted: boolean;
  privacy_notice_version: string | null;
  avatar: AvatarPreferences;
}
export interface OnboardingState {
  step: number;
  goal_text: string;
  goal_id: number | null;
  completed: boolean;
  tracking_consent: boolean;
}
export interface UserSettings {
  profile: UserProfile;
  onboarding: OnboardingState;
  autostart: boolean;
  theme: "light" | "dark";
  follow_system_theme: boolean;
}
export const defaultUserSettings: UserSettings = {
  profile: {
    name: "",
    email: "",
    role: "",
    custom_role: "",
    applications: [],
    privacy_accepted: false,
    privacy_notice_version: null,
    avatar: { character: "cat", color: "#E4FA8E", visible: true },
  },
  onboarding: {
    step: 0,
    goal_text: "",
    goal_id: null,
    completed: false,
    tracking_consent: false,
  },
  autostart: true,
  theme: "light",
  follow_system_theme: true,
};
export interface InstalledApp {
  process_name: string;
  name: string;
}
