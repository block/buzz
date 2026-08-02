import type { ClickUpTask } from "@/features/clickup/types";

export type ClickUpUrgencyGroup =
  | "overdue"
  | "today"
  | "next-seven-days"
  | "later"
  | "no-due-date";

export const CLICKUP_URGENCY_GROUPS: Array<{
  key: ClickUpUrgencyGroup;
  label: string;
}> = [
  { key: "overdue", label: "Overdue" },
  { key: "today", label: "Today" },
  { key: "next-seven-days", label: "Next 7 days" },
  { key: "later", label: "Later" },
  { key: "no-due-date", label: "No due date" },
];

export type ClickUpTaskFilters = {
  search: string;
  status: string;
  priority: string;
  location: string;
  dueWindow: "all" | ClickUpUrgencyGroup;
};

function startOfLocalDay(value: Date): number {
  return new Date(
    value.getFullYear(),
    value.getMonth(),
    value.getDate(),
  ).getTime();
}

export function taskUrgencyGroup(
  task: ClickUpTask,
  now = new Date(),
): ClickUpUrgencyGroup {
  if (!task.dueDateMs) return "no-due-date";
  const dueAt = Number(task.dueDateMs);
  if (!Number.isFinite(dueAt)) return "no-due-date";

  const today = startOfLocalDay(now);
  const tomorrow = today + 24 * 60 * 60 * 1_000;
  const afterNextSevenDays = today + 8 * 24 * 60 * 60 * 1_000;

  if (dueAt < today) return "overdue";
  if (dueAt < tomorrow) return "today";
  if (dueAt < afterNextSevenDays) return "next-seven-days";
  return "later";
}

export function taskLocationLabel(task: ClickUpTask): string {
  return [task.space?.name, task.folder?.name, task.list?.name]
    .filter(Boolean)
    .join(" › ");
}

export function filterClickUpTasks(
  tasks: ClickUpTask[],
  filters: ClickUpTaskFilters,
  now = new Date(),
): ClickUpTask[] {
  const search = filters.search.trim().toLocaleLowerCase();
  return tasks.filter((task) => {
    if (search && !task.name.toLocaleLowerCase().includes(search)) return false;
    if (filters.status !== "all" && task.status.status !== filters.status)
      return false;
    if (
      filters.priority !== "all" &&
      (task.priority?.priority ?? "none") !== filters.priority
    )
      return false;
    if (filters.location !== "all" && task.list?.id !== filters.location)
      return false;
    if (
      filters.dueWindow !== "all" &&
      taskUrgencyGroup(task, now) !== filters.dueWindow
    )
      return false;
    return true;
  });
}

export function groupClickUpTasks(
  tasks: ClickUpTask[],
  now = new Date(),
): Record<ClickUpUrgencyGroup, ClickUpTask[]> {
  const groups: Record<ClickUpUrgencyGroup, ClickUpTask[]> = {
    overdue: [],
    today: [],
    "next-seven-days": [],
    later: [],
    "no-due-date": [],
  };

  for (const task of tasks) groups[taskUrgencyGroup(task, now)].push(task);
  for (const group of Object.values(groups)) {
    group.sort((left, right) => {
      const leftDue = Number(left.dueDateMs ?? Number.POSITIVE_INFINITY);
      const rightDue = Number(right.dueDateMs ?? Number.POSITIVE_INFINITY);
      if (leftDue !== rightDue) return leftDue - rightDue;
      return left.name.localeCompare(right.name);
    });
  }
  return groups;
}

export function clickUpTaskFilterOptions(tasks: ClickUpTask[]) {
  const statuses = [...new Set(tasks.map((task) => task.status.status))]
    .filter(Boolean)
    .sort((left, right) => left.localeCompare(right));
  const priorities = [
    ...new Set(tasks.map((task) => task.priority?.priority ?? "none")),
  ].sort((left, right) => left.localeCompare(right));
  const locations = [
    ...new Map(
      tasks
        .filter((task) => task.list)
        .map((task) => [
          task.list?.id ?? "",
          { id: task.list?.id ?? "", label: taskLocationLabel(task) },
        ]),
    ).values(),
  ].sort((left, right) => left.label.localeCompare(right.label));
  return { statuses, priorities, locations };
}
