import type { PlanningSchedule } from "@/shared/api/tauriPlans";
import type { PlanningProject, PlanningTask } from "../domain/contracts";

const DAY = 86_400_000;
function offset(date: string, origin: string) {
  return Math.round(
    (Date.parse(`${date}T00:00:00Z`) - Date.parse(`${origin}T00:00:00Z`)) / DAY,
  );
}

export function GanttChart({
  project,
  tasks,
  schedule,
}: {
  project: PlanningProject;
  tasks: readonly PlanningTask[];
  schedule: PlanningSchedule;
}) {
  const byTask = new Map(schedule.tasks.map((item) => [item.taskId, item]));
  const totalDays = Math.max(
    14,
    offset(project.missionReadyDate, schedule.projectStart) + 3,
    offset(schedule.projectFinish, schedule.projectStart) + 3,
  );
  return (
    <section className="rounded-lg border bg-card" data-testid="gantt-chart">
      <div className="flex items-center justify-between border-b px-4 py-3">
        <div>
          <h2 className="text-base font-semibold">Critical path</h2>
          <p className="text-xs text-muted-foreground">
            {schedule.projectDurationWorkdays} working days · ready{" "}
            {schedule.projectFinish}
          </p>
        </div>
        <span
          className={
            schedule.missionReadyAtRisk
              ? "rounded bg-red-100 px-2 py-1 text-xs text-red-800"
              : "rounded bg-emerald-100 px-2 py-1 text-xs text-emerald-800"
          }
        >
          {schedule.missionReadyAtRisk ? "Mission ready at risk" : "On track"}
        </span>
      </div>
      <div className="overflow-x-auto p-4">
        <div className="min-w-[720px]">
          {[...tasks]
            .filter((task) => !task.isSummary)
            .sort((left, right) =>
              left.wbs.localeCompare(right.wbs, undefined, { numeric: true }),
            )
            .map((task) => {
              const result = byTask.get(task.id);
              if (!result) return null;
              const left =
                (offset(result.earliestStart, schedule.projectStart) /
                  totalDays) *
                100;
              const width =
                ((offset(result.earliestFinish, result.earliestStart) + 1) /
                  totalDays) *
                100;
              return (
                <div
                  className="grid grid-cols-[14rem_1fr] items-center gap-3 border-b py-2 last:border-0"
                  key={task.id}
                >
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">
                      {task.wbs} {task.title}
                    </p>
                    <p className="text-2xs text-muted-foreground">
                      {result.critical
                        ? "On calculated critical path"
                        : `${result.totalFloatWorkdays} working days float`}
                    </p>
                  </div>
                  <div className="relative h-8 rounded bg-muted/50">
                    <div
                      aria-label={`${task.title}: ${
                        result.critical
                          ? "critical"
                          : `${result.totalFloatWorkdays} days float`
                      }`}
                      role="img"
                      className={
                        result.critical
                          ? "absolute top-1 h-6 rounded bg-red-700"
                          : "absolute top-1 h-6 rounded bg-sky-700"
                      }
                      style={{
                        left: `${left}%`,
                        minWidth: "0.5rem",
                        width: `${Math.max(width, 1)}%`,
                      }}
                    >
                      <span
                        className="block h-full rounded bg-white/25"
                        style={{ width: `${task.percentComplete}%` }}
                      />
                    </div>
                  </div>
                </div>
              );
            })}
        </div>
      </div>
    </section>
  );
}
