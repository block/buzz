import type { PlanningTask, TaskStatus } from "./contracts";

export type KanbanColumnId =
  | "planned"
  | "ready"
  | "inProgress"
  | "waiting"
  | "forReview"
  | "complete";

export const KANBAN_COLUMNS = Object.freeze([
  { id: "planned", label: "Planned" },
  { id: "ready", label: "Ready" },
  { id: "inProgress", label: "In progress" },
  { id: "waiting", label: "Waiting" },
  { id: "forReview", label: "For review" },
  { id: "complete", label: "Complete" },
] as const);

export function kanbanColumnForTask(
  task: PlanningTask,
  tasks: readonly PlanningTask[],
): KanbanColumnId {
  if (task.status === "complete" || task.status === "cancelled")
    return "complete";
  if (task.status === "forReview") return "forReview";
  if (task.status === "blocked") return "waiting";
  if (task.status === "inProgress") return "inProgress";
  if (task.dependencyIds.length === 0) return "ready";
  const byId = new Map(tasks.map((item) => [item.id, item]));
  return task.dependencyIds.every(
    (dependencyId) => byId.get(dependencyId)?.status === "complete",
  )
    ? "ready"
    : "planned";
}

export function taskStatusForKanbanColumn(column: KanbanColumnId): TaskStatus {
  if (column === "inProgress") return "inProgress";
  if (column === "waiting") return "blocked";
  if (column === "forReview") return "forReview";
  if (column === "complete") return "complete";
  return "notStarted";
}
