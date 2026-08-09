import { localDateTimeToRfc3339 } from "../../battle-rhythm/domain/dateRange.ts";
import type { PlanningTask } from "./contracts";
import type {
  PlanningTaskDetailsV1,
  PlanningTaskExecutionV1,
} from "./extendedContracts";

export type AutomaticTaskTiming = Readonly<{
  dueAt: string;
  startAt: string;
  claimKey: string;
}>;

export type DueAutomaticTask = Readonly<{
  task: PlanningTask;
  details: PlanningTaskDetailsV1;
  timing: AutomaticTaskTiming;
  lateStart: boolean;
}>;

export function automaticStartAt(
  task: PlanningTask,
  details: PlanningTaskDetailsV1,
  timeZone: string,
): AutomaticTaskTiming {
  if (!task.dueDate) throw new Error("Automatic task has no due date.");
  const dueTime = details.dueTime ?? "16:00";
  const dueAt = localDateTimeToRfc3339(`${task.dueDate}T${dueTime}`, timeZone);
  const startAt = new Date(Date.parse(dueAt) - 60 * 60 * 1000).toISOString();
  return Object.freeze({
    dueAt,
    startAt,
    claimKey: `auto:${task.id}:${task.updatedAt}:${startAt}`,
  });
}

export function dueAutomaticTasks(
  input: Readonly<{
    tasks: readonly PlanningTask[];
    details: readonly PlanningTaskDetailsV1[];
    executions: readonly PlanningTaskExecutionV1[];
    now: string;
    timeZoneFor: (date: string) => string;
  }>,
): readonly DueAutomaticTask[] {
  const now = Date.parse(input.now);
  const detailsByTask = new Map(
    input.details.map((details) => [details.taskId, details]),
  );
  return Object.freeze(
    input.tasks
      .filter(
        (task) =>
          Boolean(task.dueDate) &&
          !["complete", "cancelled", "forReview"].includes(task.status),
      )
      .flatMap((task) => {
        const details = detailsByTask.get(task.id);
        if (
          !details ||
          details.executionMode === "manual" ||
          !details.agentId ||
          !task.dueDate
        )
          return [];
        const timing = automaticStartAt(
          task,
          details,
          input.timeZoneFor(task.dueDate),
        );
        if (Date.parse(timing.startAt) > now) return [];
        if (
          input.executions.some(
            (execution) =>
              execution.id === timing.claimKey ||
              (execution.taskId === task.id &&
                ["queued", "running", "forReview"].includes(execution.status)),
          )
        )
          return [];
        return [
          Object.freeze({
            task,
            details,
            timing,
            lateStart: now > Date.parse(timing.dueAt),
          }),
        ];
      }),
  );
}
