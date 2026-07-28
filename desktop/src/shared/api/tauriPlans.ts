import type {
  PlanningProject,
  PlanningTask,
} from "@/features/plans/domain/contracts";
import { invokeTauri } from "@/shared/api/tauri";

export type WorkingCalendar = Readonly<{
  workingWeekdays: readonly number[];
  excludedDates: readonly string[];
}>;
export type PlanningScheduleInput = Readonly<{
  project: PlanningProject;
  tasks: readonly PlanningTask[];
  workingCalendar: WorkingCalendar;
  today: string;
}>;
export type ScheduledTask = Readonly<{
  taskId: string;
  earliestStart: string;
  earliestFinish: string;
  latestStart: string;
  latestFinish: string;
  totalFloatWorkdays: number;
  critical: boolean;
  overdue: boolean;
}>;
export type PlanningSchedule = Readonly<{
  tasks: readonly ScheduledTask[];
  projectStart: string;
  projectFinish: string;
  projectDurationWorkdays: number;
  missionReadyAtRisk: boolean;
}>;

function invalid(): never {
  throw new Error("Plans returned an invalid schedule response.");
}
function record(value: unknown, keys: readonly string[]) {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid();
  const o = value as Record<string, unknown>;
  if (
    Object.keys(o).length !== keys.length ||
    Object.keys(o).some((key) => !keys.includes(key))
  )
    invalid();
  return o;
}
function date(value: unknown) {
  if (
    typeof value !== "string" ||
    !/^\d{4}-\d{2}-\d{2}$/.test(value) ||
    Number.isNaN(Date.parse(`${value}T00:00:00Z`))
  )
    invalid();
  return value;
}
function integer(value: unknown) {
  if (!Number.isSafeInteger(value)) invalid();
  return value as number;
}
function parseTask(value: unknown): ScheduledTask {
  const o = record(value, [
    "taskId",
    "earliestStart",
    "earliestFinish",
    "latestStart",
    "latestFinish",
    "totalFloatWorkdays",
    "critical",
    "overdue",
  ]);
  if (
    typeof o.taskId !== "string" ||
    !o.taskId ||
    o.taskId.length > 256 ||
    typeof o.critical !== "boolean" ||
    typeof o.overdue !== "boolean"
  )
    invalid();
  return Object.freeze({
    taskId: o.taskId,
    earliestStart: date(o.earliestStart),
    earliestFinish: date(o.earliestFinish),
    latestStart: date(o.latestStart),
    latestFinish: date(o.latestFinish),
    totalFloatWorkdays: integer(o.totalFloatWorkdays),
    critical: o.critical,
    overdue: o.overdue,
  });
}
export function parsePlanningSchedule(value: unknown): PlanningSchedule {
  const o = record(value, [
    "tasks",
    "projectStart",
    "projectFinish",
    "projectDurationWorkdays",
    "missionReadyAtRisk",
  ]);
  if (
    !Array.isArray(o.tasks) ||
    o.tasks.length > 5000 ||
    typeof o.missionReadyAtRisk !== "boolean"
  )
    invalid();
  return Object.freeze({
    tasks: Object.freeze(o.tasks.map(parseTask)),
    projectStart: date(o.projectStart),
    projectFinish: date(o.projectFinish),
    projectDurationWorkdays: integer(o.projectDurationWorkdays),
    missionReadyAtRisk: o.missionReadyAtRisk,
  });
}
export async function calculatePlanSchedule(
  input: PlanningScheduleInput,
): Promise<PlanningSchedule> {
  return parsePlanningSchedule(
    await invokeTauri("calculate_plan_schedule", { input }),
  );
}
