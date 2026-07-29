import {
  DndContext,
  type DragEndEvent,
  PointerSensor,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { GripVertical, Play } from "lucide-react";
import type { ReactNode } from "react";
import type { PlanningTask } from "../domain/contracts";
import type { PlanningTaskDetailsV1 } from "../domain/extendedContracts";
import {
  KANBAN_COLUMNS,
  type KanbanColumnId,
  kanbanColumnForTask,
  taskStatusForKanbanColumn,
} from "../domain/kanban";

function TaskCard({
  task,
  details,
  tasks,
  onEdit,
  onMove,
  onRun,
}: {
  task: PlanningTask;
  details?: PlanningTaskDetailsV1;
  tasks: readonly PlanningTask[];
  onEdit: (task: PlanningTask) => void;
  onMove: (task: PlanningTask, column: KanbanColumnId) => Promise<void>;
  onRun: (task: PlanningTask) => void;
}) {
  const column = kanbanColumnForTask(task, tasks);
  const draggable = useDraggable({
    id: task.id,
    data: { task, column },
    disabled: details?.locked,
  });
  return (
    <article
      className="rounded-lg border bg-card p-3 shadow-sm"
      data-testid={`kanban-card-${task.id}`}
      ref={draggable.setNodeRef}
      style={{
        opacity: draggable.isDragging ? 0.45 : 1,
        transform: draggable.transform
          ? `translate3d(${draggable.transform.x}px, ${draggable.transform.y}px, 0)`
          : undefined,
      }}
    >
      <div className="flex items-start justify-between gap-2">
        <button
          className="min-w-0 flex-1 text-left"
          onClick={() => onEdit(task)}
          type="button"
        >
          <span className="block text-xs text-muted-foreground">
            {task.wbs}
          </span>
          <span className="block text-sm font-medium">{task.title}</span>
        </button>
        <button
          aria-label={`Move ${task.title}`}
          className="cursor-grab rounded p-1 text-muted-foreground active:cursor-grabbing"
          type="button"
          {...draggable.attributes}
          {...draggable.listeners}
        >
          <GripVertical className="h-4 w-4" />
        </button>
      </div>
      <dl className="mt-3 grid gap-1 text-xs text-muted-foreground">
        <div className="flex justify-between gap-2">
          <dt>{details?.department ?? task.owner}</dt>
          <dd>{task.dueDate ?? "No due date"}</dd>
        </div>
        {details?.individual ? (
          <div className="truncate">Individual: {details.individual}</div>
        ) : null}
        {details?.agentId ? <div className="truncate">AI assigned</div> : null}
      </dl>
      <label className="mt-3 grid gap-1 text-xs text-muted-foreground">
        Move task
        <select
          aria-label={`Move ${task.title} to`}
          className="rounded border bg-background px-2 py-1 text-xs text-foreground"
          onChange={(event) =>
            void onMove(task, event.target.value as KanbanColumnId)
          }
          value={column}
        >
          {KANBAN_COLUMNS.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      {details?.agentId ? (
        <button
          className="mt-2 w-full rounded border px-2 py-1 text-xs font-medium"
          onClick={() => onRun(task)}
          type="button"
        >
          <Play className="mr-1 inline h-3 w-3" />
          Run adviser
        </button>
      ) : null}
    </article>
  );
}

function Column({
  id,
  label,
  children,
}: {
  id: KanbanColumnId;
  label: string;
  children: ReactNode;
}) {
  const droppable = useDroppable({ id });
  return (
    <section
      className={`min-h-64 rounded-xl border p-2 ${
        droppable.isOver ? "border-primary bg-primary/5" : "bg-muted/20"
      }`}
      data-testid={`kanban-column-${id}`}
      ref={droppable.setNodeRef}
    >
      <h3 className="px-1 py-2 text-sm font-semibold">{label}</h3>
      <div className="grid gap-2">{children}</div>
    </section>
  );
}

export function KanbanBoard({
  tasks,
  details,
  onEdit,
  onMove,
  onRun,
}: {
  tasks: readonly PlanningTask[];
  details: readonly PlanningTaskDetailsV1[];
  onEdit: (task: PlanningTask) => void;
  onMove: (task: PlanningTask, column: KanbanColumnId) => Promise<void>;
  onRun: (task: PlanningTask) => void;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );
  function dragEnded(event: DragEndEvent) {
    const target = event.over?.id as KanbanColumnId | undefined;
    const task = tasks.find((item) => item.id === event.active.id);
    if (
      !target ||
      !task ||
      !KANBAN_COLUMNS.some((column) => column.id === target) ||
      kanbanColumnForTask(task, tasks) === target
    )
      return;
    void onMove(task, target);
  }
  return (
    <DndContext onDragEnd={dragEnded} sensors={sensors}>
      <div
        className="grid min-w-[1180px] grid-cols-6 gap-3 overflow-x-auto pb-2"
        data-testid="kanban-board"
      >
        {KANBAN_COLUMNS.map((column) => (
          <Column id={column.id} key={column.id} label={column.label}>
            {tasks
              .filter((task) => kanbanColumnForTask(task, tasks) === column.id)
              .map((task) => (
                <TaskCard
                  details={details.find((item) => item.taskId === task.id)}
                  key={task.id}
                  onEdit={onEdit}
                  onMove={onMove}
                  onRun={onRun}
                  task={task}
                  tasks={tasks}
                />
              ))}
          </Column>
        ))}
      </div>
    </DndContext>
  );
}

export function moveTaskToColumn(
  task: PlanningTask,
  column: KanbanColumnId,
): PlanningTask {
  const status = taskStatusForKanbanColumn(column);
  return {
    ...task,
    status,
    percentComplete:
      status === "complete"
        ? 100
        : status === "notStarted"
          ? 0
          : Math.max(1, Math.min(task.percentComplete, 99)),
    updatedAt: new Date().toISOString(),
  };
}
