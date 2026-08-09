import type { PlanningTask } from "./contracts";

const DAY = 86_400_000;

function date(value: string) {
  const parsed = Date.parse(`${value}T00:00:00Z`);
  if (Number.isNaN(parsed)) throw new Error("Requested task date is invalid.");
  return parsed;
}

function shifted(value: string, days: number) {
  return new Date(date(value) + days * DAY).toISOString().slice(0, 10);
}

export function requestTaskMove(
  task: PlanningTask,
  targetDate: string,
  locked: boolean,
  updatedAt = new Date().toISOString(),
): PlanningTask {
  if (locked) throw new Error("This task is locked against rescheduling.");
  const currentStart = task.plannedStart ?? targetDate;
  const delta = Math.round((date(targetDate) - date(currentStart)) / DAY);
  return {
    ...task,
    plannedStart: targetDate,
    dueDate: task.dueDate ? shifted(task.dueDate, delta) : targetDate,
    fixedStart: targetDate,
    updatedAt,
  };
}
