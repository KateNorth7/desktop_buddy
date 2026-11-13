import { useMemo, useState } from "react";
import { Check, ChevronLeft, ChevronRight } from "lucide-react";
import { PeriodPicker, endOfMonth } from "./PeriodPicker";
import {
  sentenceCase,
  dateLabel,
  duration,
  shiftDate,
  type CoreSnapshot,
  type DayProgress,
  type ProgressPeriod,
} from "./types";

const trackingIssues: Record<string, string> = {
  skipped_lane_busy: "Goal matching often waited for other AI checks.",
  skipped_hourly_budget: "The hourly AI limit was reached.",
  provider_rate_limit: "The built-in AI allowance ran out.",
  provider_timeout_or_network: "The AI service timed out.",
  provider_error: "The AI service returned errors.",
  answered_no_goal: "The AI found no matching goal.",
  answered_below_threshold: "The AI was not sure enough to assign it.",
};

/** Recorded time, the part that reached a goal, and why the rest is still waiting. */
export function trackingLine(days: DayProgress[]): string | null {
  let recorded = 0;
  let attributed = 0;
  let waiting = 0;
  const issues = new Map<string, number>();
  for (const day of days) {
    const tracking = day.tracking;
    if (!tracking) continue;
    recorded += tracking.recorded_seconds;
    attributed += tracking.attributed_seconds;
    waiting += tracking.waiting_seconds;
    if (tracking.issue && tracking.waiting_seconds > 0)
      issues.set(
        tracking.issue,
        (issues.get(tracking.issue) ?? 0) + tracking.waiting_seconds,
      );
  }
  if (recorded <= 0) return null;
  const parts = [
    `Recorded ${duration(recorded)}`,
    `${duration(attributed)} matched to goals`,
  ];
  if (waiting > 0) parts.push(`${duration(waiting)} waiting for a goal`);
  const issue = [...issues.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
  const reason = issue ? trackingIssues[issue] : undefined;
  return `${parts.join(" · ")}${reason ? `. ${reason}` : ""}`;
}

const periodOptions: Array<{
  value: ProgressPeriod;
  label: string;
  days: number;
}> = [
  { value: "day", label: "Day", days: 1 },
  { value: "week", label: "Week", days: 7 },
  { value: "month", label: "Month", days: 31 },
  { value: "year", label: "Year", days: 366 },
];

const periodDays = (period: ProgressPeriod) =>
  periodOptions.find((item) => item.value === period)?.days ?? 7;

export function DaySummary({ day }: { day: DayProgress; unfinished?: number }) {
  const seconds = day.goals.reduce(
    (sum, goal) => sum + (goal.relevant_seconds ?? 0),
    0,
  );
  const closedGoals = day.completed_goals?.length ?? 0;
  const goalsDone = closedGoals;
  const stepsDone = Math.max(0, day.completed.length - closedGoals);
  const parts = [
    stepsDone > 0
      ? `${stepsDone} ${stepsDone === 1 ? "step" : "steps"} done`
      : "",
    goalsDone > 0
      ? `${goalsDone} ${goalsDone === 1 ? "goal" : "goals"} completed`
      : "",
    seconds > 0 ? `${duration(seconds)} focused` : "",
  ].filter(Boolean);
  return (
    <section
      className={`day-summary ${parts.length ? "has-progress" : ""}`}
      aria-label="Day summary"
    >
      <Check size={16} aria-hidden="true" />
      <p>
        {parts.length ? (
          <strong>{parts.join(" · ")}</strong>
        ) : (
          "No recorded activity or completed goals for this day."
        )}
      </p>
    </section>
  );
}
export interface FinishedItem {
  id: number;
  title: string;
  seconds: number;
  /** Steps finished today; for a completed goal, all of its done steps. */
  steps: string[];
  completed: boolean;
}
/** Today's callout: focused time first, finished goals and steps on the right. */
export function TodaySummary({
  day,
  items,
}: {
  day: DayProgress;
  items: FinishedItem[];
}) {
  const seconds = day.goals.reduce(
    (sum, goal) => sum + (goal.relevant_seconds ?? 0),
    0,
  );
  const goalsDone = items.filter((item) => item.completed).length;
  const stepsDone = items.reduce((sum, item) => sum + item.steps.length, 0);
  const counts = [
    goalsDone > 0 ? `${goalsDone} ${goalsDone === 1 ? "goal" : "goals"}` : "",
    stepsDone > 0 ? `${stepsDone} ${stepsDone === 1 ? "step" : "steps"}` : "",
  ].filter(Boolean);
  const headline =
    seconds > 0
      ? `${duration(seconds)} of focused work today`
      : "No focused work recorded yet today";
  return (
    <section className="today-summary" aria-label="Day summary">
      <div className="today-summary-head">
        <strong>{headline}</strong>
        {counts.length > 0 && (
          <span className="today-summary-counts">{counts.join(" · ")}</span>
        )}
      </div>
    </section>
  );
}
// Shades describe time rank in the displayed period, rather than goal identity.
// Keep the brand violet at the top and stop short of white so small segments stay visible.
function violetShade(rank: number, count: number) {
  const violet = count > 1 ? 100 - (rank / (count - 1)) * 55 : 100;
  return `color-mix(in srgb, #6c5fff ${violet.toFixed(3)}%, #ffffff)`;
}

type Bucket = {
  key: string;
  label: string;
  goals: Map<number, number>;
};

function minutes(seconds: number) {
  return seconds > 0 && seconds < 60 ? "<1m" : duration(seconds);
}

function bucketsFor(days: DayProgress[], period: ProgressPeriod): Bucket[] {
  const buckets = new Map<string, Bucket>();
  for (const day of days) {
    const key = period === "year" ? day.day.slice(0, 7) : day.day;
    const bucket = buckets.get(key) ?? {
      key,
      label:
        period === "year"
          ? new Date(`${key}-01T12:00:00`).toLocaleDateString("en", {
              month: "short",
            })
          : period === "week"
            ? new Date(`${day.day}T12:00:00`).toLocaleDateString("en", {
                weekday: "short",
              })
            : dateLabel(day.day).replace(/^[A-Za-z]+, /, ""),
      goals: new Map<number, number>(),
    };
    for (const goal of day.goals)
      bucket.goals.set(
        goal.goal_id,
        (bucket.goals.get(goal.goal_id) ?? 0) + (goal.relevant_seconds ?? 0),
      );
    buckets.set(key, bucket);
  }
  return [...buckets.values()];
}

function hourBuckets(hourly: CoreSnapshot["hourly"]): Bucket[] {
  return Array.from({ length: 24 }, (_, hour) => ({
    key: String(hour),
    label: hour % 6 === 0 ? `${String(hour).padStart(2, "0")}:00` : "",
    goals: new Map(
      hourly
        .filter((item) => item.hour === hour)
        .map((item) => [item.goal_id, item.seconds] as const),
    ),
  }));
}

export function Progress({
  snapshot,
  anchor,
  period,
  onWeek,
  onPeriod,
  busy,
}: {
  snapshot: CoreSnapshot;
  anchor: string | null;
  period: ProgressPeriod;
  onWeek: (anchor: string | null) => void;
  onPeriod: (period: ProgressPeriod) => void;
  busy: boolean;
}) {
  const [selected, setSelected] = useState<number | null>(null);
  const [hover, setHover] = useState<{ bucket: string; goal: number } | null>(
    null,
  );
  const days = useMemo(
    () =>
      snapshot.week.length ? [...snapshot.week].reverse() : [snapshot.summary],
    [snapshot.week, snapshot.summary],
  );
  const newest = anchor ?? snapshot.date;
  const oldest = days[0]?.day ?? snapshot.date;
  const goals = useMemo(() => {
    const totals = new Map<
      number,
      { id: number; title: string; seconds: number }
    >();
    for (const day of days)
      for (const item of day.goals) {
        const row = totals.get(item.goal_id) ?? {
          id: item.goal_id,
          title: item.title,
          seconds: 0,
        };
        row.seconds += item.relevant_seconds ?? 0;
        totals.set(item.goal_id, row);
      }
    return [...totals.values()]
      .filter((goal) => goal.seconds > 0)
      .sort((a, b) => b.seconds - a.seconds || a.id - b.id);
  }, [days]);
  const timeRanks = new Map(
    [...new Set(goals.map((goal) => goal.seconds))].map((seconds, rank) => [
      seconds,
      rank,
    ]),
  );
  const goalRanks = new Map(goals.map((goal, rank) => [goal.id, rank]));
  const goalColors = new Map(
    goals.map((goal) => [
      goal.id,
      violetShade(timeRanks.get(goal.seconds) ?? 0, timeRanks.size),
    ]),
  );
  const colorOf = (id: number) => goalColors.get(id) ?? violetShade(0, 1);
  const focus =
    selected === null
      ? null
      : (snapshot.goals.find((g) => g.id === selected) ??
        (goals.find((g) => g.id === selected)
          ? { ...goals.find((g) => g.id === selected)!, plan: { steps: [] } }
          : null));
  const titleOf = (id: number) =>
    sentenceCase(
      snapshot.goals.find((g) => g.id === id)?.title ??
        goals.find((g) => g.id === id)?.title ??
        "Goal",
    );
  const buckets = (
    period === "day"
      ? hourBuckets(snapshot.hourly ?? [])
      : bucketsFor(days, period)
  ).map((bucket) => ({
    ...bucket,
    goals:
      selected === null
        ? bucket.goals
        : new Map([[selected, bucket.goals.get(selected) ?? 0]]),
  }));
  const bucketTotal = (bucket: Bucket) =>
    [...bucket.goals.values()].reduce((sum, value) => sum + value, 0);
  const maxBucket = Math.max(1, ...buckets.map(bucketTotal));
  // Keep tiny segments visible, then scale every stack equally to fit the plot.
  const rawHeight = (seconds: number) =>
    Math.max(6, (seconds / maxBucket) * 110);
  const entries = (bucket: Bucket) =>
    [...bucket.goals.entries()]
      .filter(([, seconds]) => seconds > 0)
      .sort(
        ([a], [b]) =>
          (goalRanks.get(a) ?? goals.length) -
            (goalRanks.get(b) ?? goals.length) || a - b,
      );
  const maxHeight = Math.max(
    110,
    ...buckets.map((bucket) => {
      const rows = entries(bucket);
      return (
        rows.reduce((sum, [, seconds]) => sum + rawHeight(seconds), 0) +
        Math.max(0, rows.length - 1) * 2
      );
    }),
  );
  const scale = 110 / maxHeight;
  const total = buckets.reduce((sum, bucket) => sum + bucketTotal(bucket), 0);
  const stepSeconds = new Map<string, number>();
  if (selected !== null)
    for (const day of days)
      for (const item of day.goals)
        if (item.goal_id === selected)
          for (const step of item.steps ?? [])
            stepSeconds.set(
              step.step_id,
              (stepSeconds.get(step.step_id) ?? 0) + step.seconds,
            );
  const doneSteps = focus?.plan.steps.filter((step) => step.done) ?? [];
  const allSteps = focus?.plan.steps ?? [];
  const periodGoalsDone = days.reduce(
    (sum, day) => sum + (day.completed_goals?.length ?? 0),
    0,
  );
  const periodStepsDone = days.reduce(
    (sum, day) =>
      sum +
      Math.max(0, day.completed.length - (day.completed_goals?.length ?? 0)),
    0,
  );
  const periodWord =
    period === "day"
      ? "today"
      : period === "week"
        ? "this week"
        : period === "month"
          ? "this month"
          : "this year";
  const listed = doneSteps.reduce(
    (sum, step) => sum + (stepSeconds.get(step.id) ?? 0),
    0,
  );
  const other = Math.max(0, total - listed);
  const tracking = trackingLine(days);
  const step = periodDays(period);
  const shiftPeriod = (direction: -1 | 1) => {
    const base = new Date(`${newest}T12:00:00`);
    const target =
      period === "month"
        ? endOfMonth(base.getFullYear(), base.getMonth() + direction)
        : period === "year"
          ? `${base.getFullYear() + direction}-12-31`
          : shiftDate(newest, direction * step);
    return target >= snapshot.date ? null : target;
  };
  const shortDate = (value: string) =>
    dateLabel(value).replace(/^[A-Za-z]+, /, "");
  const rangeLabel =
    period === "day"
      ? dateLabel(newest)
      : period === "month"
        ? new Date(`${newest}T12:00:00`).toLocaleDateString("en", {
            month: "long",
            year: "numeric",
          })
        : period === "year"
          ? newest.slice(0, 4)
          : `${shortDate(oldest)} – ${shortDate(newest)}`;
  return (
    <section
      className="core-section progress-history progress-screen"
      aria-label="Progress"
    >
      {focus && (
        <div className="progress-top">
          <button
            className="text-button progress-back"
            onClick={() => setSelected(null)}
          >
            <ChevronLeft size={16} />
            Progress
          </button>
        </div>
      )}
      {focus && (
        <h2 className="progress-goal-title">{sentenceCase(focus.title)}</h2>
      )}
      <div className="progress-headline">
        <div className="progress-total">
          {total > 0 ? duration(total) : "0m"}
        </div>
        <p className="progress-sub">
          {focus
            ? `focused ${periodWord} · ${doneSteps.length} of ${allSteps.length} ${allSteps.length === 1 ? "step" : "steps"} done`
            : [
                `focused ${periodWord}`,
                periodGoalsDone > 0
                  ? `${periodGoalsDone} ${periodGoalsDone === 1 ? "goal" : "goals"}`
                  : "",
                periodStepsDone > 0
                  ? `${periodStepsDone} ${periodStepsDone === 1 ? "step" : "steps"} done`
                  : "",
              ]
                .filter(Boolean)
                .join(" · ")}
        </p>
        {!focus && tracking && (
          <p className="progress-sub" aria-label="Tracking status">
            {tracking}
          </p>
        )}
      </div>
      <div className="progress-bar">
        <div className="segmented-control" role="tablist" aria-label="Period">
          {periodOptions.map((item) => (
            <button
              key={item.value}
              role="tab"
              aria-selected={period === item.value}
              className={period === item.value ? "selected" : ""}
              disabled={busy}
              onClick={() => {
                onWeek(null);
                onPeriod(item.value);
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
        <div className="progress-nav" role="group" aria-label="Choose period">
          <button
            className="text-button"
            aria-label={`Previous ${period}`}
            disabled={busy}
            onClick={() => onWeek(shiftPeriod(-1))}
          >
            <ChevronLeft size={16} />
          </button>
          <PeriodPicker
            period={period}
            newest={newest}
            today={snapshot.date}
            label={rangeLabel}
            disabled={busy}
            onPick={onWeek}
          />
          <button
            className="text-button"
            aria-label={`Next ${period}`}
            disabled={busy || newest >= snapshot.date}
            onClick={() => onWeek(shiftPeriod(1))}
          >
            <ChevronRight size={16} />
          </button>
        </div>
      </div>
      <div className="stack-chart" aria-label="Focused time">
        {buckets.map((bucket) => (
          <div key={bucket.key} className="stack-col">
            {hover?.bucket === bucket.key && bucket.goals.has(hover.goal) && (
              <div
                className="stack-tip"
                role="tooltip"
                id={`time-${bucket.key}-${hover.goal}`}
              >
                <strong>{titleOf(hover.goal)}</strong>
                <span>
                  {period === "day"
                    ? `${bucket.key.padStart(2, "0")}:00`
                    : bucket.label}{" "}
                  · {minutes(bucket.goals.get(hover.goal)!)}
                </span>
              </div>
            )}
            <div className="stack-bar" style={{ gap: `${2 * scale}px` }}>
              {entries(bucket).map(([id, seconds]) => (
                <button
                  key={id}
                  type="button"
                  className="stack-segment"
                  aria-label={`${titleOf(id)} · ${period === "day" ? `${bucket.key.padStart(2, "0")}:00` : bucket.label} · ${minutes(seconds)}`}
                  aria-describedby={
                    hover?.bucket === bucket.key && hover.goal === id
                      ? `time-${bucket.key}-${id}`
                      : undefined
                  }
                  onMouseEnter={() =>
                    setHover({ bucket: bucket.key, goal: id })
                  }
                  onMouseLeave={() => setHover(null)}
                  onFocus={() => setHover({ bucket: bucket.key, goal: id })}
                  onBlur={() => setHover(null)}
                  onClick={() => {
                    setSelected(id);
                    setHover(null);
                  }}
                  style={
                    {
                      height: `${rawHeight(seconds) * scale}px`,
                      "--bar-color": colorOf(id),
                    } as React.CSSProperties
                  }
                />
              ))}
            </div>
            <small>
              {buckets.length > 14 && period !== "day" ? "" : bucket.label}
            </small>
          </div>
        ))}
      </div>
      {!focus &&
        (goals.length === 0 ? (
          <p className="empty-copy">No focused time in this period yet.</p>
        ) : (
          <ul className="progress-list">
            {goals.map((goal) => (
              <li key={goal.id}>
                <button
                  className="text-button progress-row"
                  onClick={() => setSelected(goal.id)}
                >
                  <i style={{ background: colorOf(goal.id) }} />
                  <span>{sentenceCase(goal.title)}</span>
                  <em>{duration(goal.seconds)}</em>
                  <ChevronRight size={16} />
                </button>
              </li>
            ))}
          </ul>
        ))}
      {focus && (
        <ul className="progress-list">
          {allSteps.map((item) => (
            <li
              key={item.id}
              className={`progress-row static ${item.done ? "" : "open"}`}
            >
              <span
                className={`done-box ${item.done ? "" : "is-open"}`}
                aria-hidden="true"
              >
                {item.done && <Check size={10} />}
              </span>
              <span>{sentenceCase(item.text)}</span>
              <em>
                {stepSeconds.get(item.id)
                  ? minutes(stepSeconds.get(item.id)!)
                  : "–"}
              </em>
            </li>
          ))}
          {other > 0 && (
            <li className="progress-row static muted">
              <span />
              <span>Other work on this goal</span>
              <em>{minutes(other)}</em>
            </li>
          )}
          {allSteps.length === 0 && other === 0 && (
            <li className="empty-copy">
              No steps or focused time in this period.
            </li>
          )}
        </ul>
      )}
    </section>
  );
}
