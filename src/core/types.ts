import type { GoalPlan } from "../types";
export interface GoalDraft {
  title: string;
  area: string;
  steps: string[];
}
export interface CoreGoal {
  id: number;
  title: string;
  status: "open" | "deferred" | "completed";
  area_id: number;
  plan: GoalPlan;
  focused_seconds: number;
  tracked_seconds?: number;
  relevant_seconds?: number;
  due_at?: string | null;
  priority?: "low" | "medium" | "high" | "urgent" | null;
  description?: string;
  created_at?: string;
  completed_at?: string | null;
  estimated_minutes?: number | null;
  work_context?: import("../api/memory").WorkContext | null;
  analysis?: {
    state: "" | "queued" | "working" | "ready" | "failed";
    result: import("./analysis").GoalAnalysisResult | null;
    message: string | null;
  };
}
export interface GoalDetails {
  due_at: string | null;
  priority: "low" | "medium" | "high" | "urgent" | null;
  description: string;
}
export interface CorePreferences {
  working_days: number[];
  movement_reminders: boolean;
  movement_interval_minutes?: number;
  vision_model: string;
  ai_connection_mode?: "built_in" | "personal";
}
interface GoalTime {
  goal_id: number;
  title: string;
  area: string;
  seconds: number;
  tracked_seconds?: number;
  relevant_seconds?: number;
  steps?: Array<{ step_id: string; seconds: number }>;
}
/** Recorded activity for a day and how much of it reached a goal. */
export interface TrackingDay {
  recorded_seconds: number;
  attributed_seconds: number;
  waiting_seconds: number;
  /** Stable code for the most frequent reason time stayed unassigned. */
  issue?: string | null;
}
export interface DayProgress {
  day: string;
  completed: string[];
  completed_goals?: string[];
  goals: GoalTime[];
  tracking?: TrackingDay;
}
export interface CoreSnapshot {
  onboarding_goal_id?: number | null;
  date: string;
  selected_date: string;
  date_goals: number[];
  carryover: number[];
  day_mode: "unset" | "plan" | "no_plan";
  today: number[];
  areas: { id: number; title: string }[];
  goals: CoreGoal[];
  summary: DayProgress;
  week: DayProgress[];
  hourly: Array<{ hour: number; goal_id: number; seconds: number }>;
  timer: { goal_id: number; last_tick: string } | null;
  focused_goal_id: number | null;
  preferences: CorePreferences;
}
export interface ImageInput {
  mime: string;
  data: string;
}
export interface ImportProposal {
  reply: string;
  drafts: GoalDraft[];
}
export type ProgressPeriod = "day" | "week" | "month" | "year";
export const emptyCore = (): CoreSnapshot => {
  const date = new Date().toLocaleDateString("en-CA");
  const summary: DayProgress = { day: date, completed: [], goals: [] };
  return {
    date,
    selected_date: date,
    date_goals: [],
    carryover: [],
    day_mode: "unset",
    today: [],
    areas: [],
    goals: [],
    summary,
    week: Array.from({ length: 7 }, (_, index) => ({
      day: shiftDate(date, -index),
      completed: [],
      goals: [],
    })),
    hourly: [],
    timer: null,
    focused_goal_id: null,
    preferences: {
      working_days: [1, 2, 3, 4, 5],
      movement_reminders: true,
      movement_interval_minutes: 45,
      vision_model: "",
      ai_connection_mode: "built_in",
    },
  };
};
export function duration(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}h ${minutes % 60}m`
    : minutes > 0
      ? `${minutes}m`
      : seconds > 0
        ? "<1m"
        : "0m";
}
export function localDeadline(value?: string | null): string {
  if (!value) return "";
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "";
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}T${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}
export function deadlineISO(value: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (!Number.isFinite(date.getTime()))
    throw new Error("Choose a valid deadline");
  return date.toISOString();
}
export function dateLabel(day: string): string {
  return new Date(`${day}T12:00:00`).toLocaleDateString("en", {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}
export function shiftDate(day: string, days: number): string {
  const date = new Date(`${day}T12:00:00`);
  date.setDate(date.getDate() + days);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

/** Goals and steps read like sentences, including ones saved before capitalising became automatic. */
export function sentenceCase(text: string) {
  const first = text.charAt(0);
  return first && first !== first.toLocaleUpperCase()
    ? first.toLocaleUpperCase() + text.slice(1)
    : text;
}
