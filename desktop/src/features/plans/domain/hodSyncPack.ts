import type { ScheduledTask } from "@/shared/api/tauriPlans";
import type { PlanningProject, PlanningTask } from "./contracts";
import type { PlanningTaskDetailsV1 } from "./extendedContracts";

export type HodGroup = "XO" | "MEO" | "WEEO" | "SO" | "other";

export type HodSyncItem = Readonly<{
  task: PlanningTask;
  details: PlanningTaskDetailsV1;
  schedule: ScheduledTask | null;
  overdue: boolean;
  critical: boolean;
  incompleteDependencies: readonly string[];
}>;

export type HodSyncPack = Readonly<{
  project: PlanningProject;
  generatedAt: string;
  groups: Readonly<Record<HodGroup, readonly HodSyncItem[]>>;
  combined: readonly HodSyncItem[];
}>;

const HOD_GROUPS = ["XO", "MEO", "WEEO", "SO"] as const;

function groupFor(details: PlanningTaskDetailsV1): HodGroup {
  return HOD_GROUPS.includes(details.department as (typeof HOD_GROUPS)[number])
    ? (details.department as (typeof HOD_GROUPS)[number])
    : "other";
}

function compare(left: HodSyncItem, right: HodSyncItem) {
  if (left.overdue !== right.overdue) return left.overdue ? -1 : 1;
  if (left.critical !== right.critical) return left.critical ? -1 : 1;
  const due = (left.task.dueDate ?? "9999-12-31").localeCompare(
    right.task.dueDate ?? "9999-12-31",
  );
  return (
    due ||
    left.task.wbs.localeCompare(right.task.wbs, undefined, {
      numeric: true,
    })
  );
}

export function buildHodSyncPack(
  project: PlanningProject,
  tasks: readonly PlanningTask[],
  details: readonly PlanningTaskDetailsV1[],
  schedule: readonly ScheduledTask[],
  generatedAt: string,
): HodSyncPack {
  const detailByTask = new Map(details.map((item) => [item.taskId, item]));
  const scheduleByTask = new Map(schedule.map((item) => [item.taskId, item]));
  const taskById = new Map(tasks.map((item) => [item.id, item]));
  const today = generatedAt.slice(0, 10);
  const groups: Record<HodGroup, HodSyncItem[]> = {
    XO: [],
    MEO: [],
    WEEO: [],
    SO: [],
    other: [],
  };
  for (const task of tasks.filter(
    (item) =>
      !item.isSummary &&
      item.status !== "complete" &&
      item.status !== "cancelled",
  )) {
    const taskDetails = detailByTask.get(task.id);
    if (!taskDetails) continue;
    const scheduled = scheduleByTask.get(task.id) ?? null;
    const item: HodSyncItem = Object.freeze({
      task,
      details: taskDetails,
      schedule: scheduled,
      overdue: Boolean(task.dueDate && task.dueDate < today),
      critical: scheduled?.critical ?? false,
      incompleteDependencies: Object.freeze(
        task.dependencyIds
          .map((id) => taskById.get(id))
          .filter(
            (dependency): dependency is PlanningTask =>
              Boolean(dependency) && dependency?.status !== "complete",
          )
          .map((dependency) => dependency.title),
      ),
    });
    groups[groupFor(taskDetails)].push(item);
  }
  for (const group of Object.values(groups)) group.sort(compare);
  const frozen = Object.freeze({
    XO: Object.freeze(groups.XO),
    MEO: Object.freeze(groups.MEO),
    WEEO: Object.freeze(groups.WEEO),
    SO: Object.freeze(groups.SO),
    other: Object.freeze(groups.other),
  });
  return Object.freeze({
    project,
    generatedAt,
    groups: frozen,
    combined: Object.freeze([
      ...frozen.XO,
      ...frozen.MEO,
      ...frozen.WEEO,
      ...frozen.SO,
      ...frozen.other,
    ]),
  });
}
