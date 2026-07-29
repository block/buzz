import {
  DndContext,
  type DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { GripHorizontal } from "lucide-react";
import type { PlanningSchedule } from "@/shared/api/tauriPlans";
import type { PlanningProject, PlanningTask } from "../domain/contracts";

const DAY = 86_400_000;

function offset(date: string, origin: string) {
  return Math.round(
    (Date.parse(`${date}T00:00:00Z`) - Date.parse(`${origin}T00:00:00Z`)) / DAY,
  );
}

function dateAt(origin: string, day: number) {
  return new Date(Date.parse(`${origin}T00:00:00Z`) + day * DAY)
    .toISOString()
    .slice(0, 10);
}

function DayDrop({ date }: { date: string }) {
  const drop = useDroppable({ id: `gantt-day:${date}` });
  return (
    <div
      className={`h-full border-r last:border-r-0 ${
        drop.isOver ? "bg-primary/20" : ""
      }`}
      data-testid={`gantt-day-${date}`}
      ref={drop.setNodeRef}
    />
  );
}

function TaskBar({
  task,
  critical,
  left,
  width,
  locked,
}: {
  task: PlanningTask;
  critical: boolean;
  left: number;
  width: number;
  locked: boolean;
}) {
  const drag = useDraggable({
    id: `gantt-task:${task.id}`,
    data: { taskId: task.id },
    disabled: locked,
  });
  return (
    <button
      aria-label={
        locked
          ? `${task.title} is locked against rescheduling`
          : `Drag ${task.title} to reschedule`
      }
      className={`absolute top-1 h-6 rounded text-white ${
        critical ? "bg-red-700" : "bg-sky-700"
      } ${locked ? "cursor-not-allowed opacity-70" : "cursor-grab active:cursor-grabbing"}`}
      ref={drag.setNodeRef}
      style={{
        left: `${left}%`,
        minWidth: "0.75rem",
        transform: drag.transform
          ? `translate3d(${drag.transform.x}px, ${drag.transform.y}px, 0)`
          : undefined,
        width: `${Math.max(width, 1)}%`,
        zIndex: drag.isDragging ? 10 : 1,
      }}
      title={locked ? "Locked against rescheduling" : "Drag to a new date"}
      type="button"
      {...drag.attributes}
      {...drag.listeners}
    >
      <span
        className="block h-full rounded bg-white/25"
        style={{ width: `${task.percentComplete}%` }}
      />
      <GripHorizontal className="absolute left-1 top-1 h-4 w-4" />
    </button>
  );
}

export function GanttChart({
  project,
  tasks,
  schedule,
  lockedTaskIds = new Set(),
  onRequestMove,
}: {
  project: PlanningProject;
  tasks: readonly PlanningTask[];
  schedule: PlanningSchedule;
  lockedTaskIds?: ReadonlySet<string>;
  onRequestMove?: (task: PlanningTask, targetDate: string) => void;
}) {
  const byTask = new Map(schedule.tasks.map((item) => [item.taskId, item]));
  const totalDays = Math.max(
    14,
    offset(project.missionReadyDate, schedule.projectStart) + 3,
    offset(schedule.projectFinish, schedule.projectStart) + 3,
  );
  const days = Array.from({ length: totalDays }, (_, index) =>
    dateAt(schedule.projectStart, index),
  );
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor),
  );
  function dragEnded(event: DragEndEvent) {
    const taskId = String(event.active.id).replace("gantt-task:", "");
    const target = String(event.over?.id ?? "");
    if (!target.startsWith("gantt-day:")) return;
    const task = tasks.find((item) => item.id === taskId);
    if (task) onRequestMove?.(task, target.replace("gantt-day:", ""));
  }
  return (
    <DndContext onDragEnd={dragEnded} sensors={sensors}>
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
          <p className="mb-2 text-xs text-muted-foreground">
            Drag an unlocked bar to a date, or use its date control. Changes
            remain a preview until Apply.
          </p>
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
                const locked = lockedTaskIds.has(task.id);
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
                      <label className="mt-1 flex items-center gap-1 text-2xs text-muted-foreground">
                        Move to
                        <input
                          aria-label={`Move ${task.title} to date`}
                          className="rounded border bg-background px-1 py-0.5"
                          disabled={locked}
                          onChange={(event) =>
                            onRequestMove?.(task, event.target.value)
                          }
                          type="date"
                          value={result.earliestStart}
                        />
                      </label>
                    </div>
                    <div className="relative h-8 rounded bg-muted/50">
                      <div
                        className="absolute inset-0 grid"
                        style={{
                          gridTemplateColumns: `repeat(${totalDays}, minmax(0, 1fr))`,
                        }}
                      >
                        {days.map((date) => (
                          <DayDrop date={date} key={date} />
                        ))}
                      </div>
                      <TaskBar
                        critical={result.critical}
                        left={left}
                        locked={locked}
                        task={task}
                        width={width}
                      />
                    </div>
                  </div>
                );
              })}
          </div>
        </div>
      </section>
    </DndContext>
  );
}
