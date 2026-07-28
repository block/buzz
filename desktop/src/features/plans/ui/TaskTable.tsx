import type { ScheduledTask } from "@/shared/api/tauriPlans";
import type { PlanningTask } from "../domain/contracts";

export function TaskTable({
  tasks,
  schedule,
  onEdit,
}: {
  tasks: readonly PlanningTask[];
  schedule: readonly ScheduledTask[];
  onEdit: (task: PlanningTask) => void;
}) {
  const calculated = new Map(schedule.map((item) => [item.taskId, item]));
  return (
    <div className="overflow-x-auto rounded-lg border">
      <table className="w-full min-w-[760px] text-left text-sm">
        <thead className="bg-muted/50 text-xs uppercase text-muted-foreground">
          <tr>
            <th className="px-3 py-2">WBS</th>
            <th className="px-3 py-2">Task</th>
            <th className="px-3 py-2">Owner</th>
            <th className="px-3 py-2">Due</th>
            <th className="px-3 py-2">Progress</th>
            <th className="px-3 py-2">Path</th>
          </tr>
        </thead>
        <tbody>
          {[...tasks]
            .sort((left, right) =>
              left.wbs.localeCompare(right.wbs, undefined, { numeric: true }),
            )
            .map((task) => {
              const result = calculated.get(task.id);
              return (
                <tr
                  className="cursor-pointer border-t hover:bg-muted/40"
                  key={task.id}
                  onClick={() => onEdit(task)}
                >
                  <td className="px-3 py-2 font-mono text-xs">{task.wbs}</td>
                  <td className="px-3 py-2 font-medium">{task.title}</td>
                  <td className="px-3 py-2">{task.owner}</td>
                  <td className="px-3 py-2">{task.dueDate ?? "—"}</td>
                  <td className="px-3 py-2">{task.percentComplete}%</td>
                  <td className="px-3 py-2">
                    {result?.critical ? (
                      <span className="font-medium text-red-700 dark:text-red-300">
                        Critical
                      </span>
                    ) : result ? (
                      `${result.totalFloatWorkdays}d float`
                    ) : (
                      "Pending"
                    )}
                  </td>
                </tr>
              );
            })}
        </tbody>
      </table>
    </div>
  );
}
