import { AlertTriangle, Plus } from "lucide-react";
import type { ScheduledTask } from "@/shared/api/tauriPlans";
import type { MissionConstraint, PlanningTask } from "../domain/contracts";

const resolved = new Set(["resolved", "missionChanged"]);
export function MissionConstraintsPanel({
  constraints,
  tasks,
  schedule,
  onCreate,
  onEdit,
}: {
  constraints: readonly MissionConstraint[];
  tasks: readonly PlanningTask[];
  schedule: readonly ScheduledTask[];
  onCreate: () => void;
  onEdit: (constraint: MissionConstraint) => void;
}) {
  const taskById = new Map(tasks.map((task) => [task.id, task]));
  const scheduleById = new Map(schedule.map((task) => [task.taskId, task]));
  const ordered = [...constraints].sort((left, right) => {
    const leftOpen = resolved.has(left.status) ? 1 : 0;
    const rightOpen = resolved.has(right.status) ? 1 : 0;
    if (leftOpen !== rightOpen) return leftOpen - rightOpen;
    const rank = { critical: 0, high: 1, medium: 2, low: 3 };
    return rank[left.severity] - rank[right.severity];
  });
  return (
    <section
      className="rounded-lg border bg-card"
      data-testid="mission-constraints"
    >
      <div className="flex items-center justify-between border-b px-4 py-3">
        <div>
          <h2 className="text-base font-semibold">Mission constraints</h2>
          <p className="text-xs text-muted-foreground">
            Defects and conditions remain open until explicitly dispositioned.
          </p>
        </div>
        <button
          className="rounded border px-3 py-2 text-sm"
          onClick={onCreate}
          type="button"
        >
          <Plus className="mr-1 inline h-4 w-4" />
          Add constraint
        </button>
      </div>
      {ordered.length ? (
        <div className="grid gap-2 p-4">
          {ordered.map((constraint) => {
            const task = constraint.linkedTaskId
              ? taskById.get(constraint.linkedTaskId)
              : undefined;
            const path = task ? scheduleById.get(task.id) : undefined;
            return (
              <button
                className="rounded border p-3 text-left hover:bg-muted/40"
                key={constraint.id}
                onClick={() => onEdit(constraint)}
                type="button"
              >
                <div className="flex items-start gap-2">
                  <AlertTriangle
                    className={
                      constraint.severity === "critical"
                        ? "mt-0.5 h-4 w-4 text-red-700"
                        : "mt-0.5 h-4 w-4 text-amber-700"
                    }
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <strong className="text-sm">
                        {constraint.description}
                      </strong>
                      <span className="rounded bg-muted px-1.5 py-0.5 text-2xs uppercase">
                        {constraint.status}
                      </span>
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {task ? `Linked task: ${task.wbs} ${task.title}. ` : ""}
                      {path?.critical
                        ? "On calculated critical path."
                        : task
                          ? "Mission-critical constraint outside calculated path."
                          : ""}
                    </p>
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      ) : (
        <p className="p-4 text-sm text-muted-foreground">
          No mission constraints have been recorded.
        </p>
      )}
    </section>
  );
}
