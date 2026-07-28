import * as React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  parsePlanningTask,
  type PlanningTask,
  type TaskStatus,
} from "../domain/contracts";

export function TaskEditorDialog({
  open,
  onOpenChange,
  projectId,
  task,
  tasks,
  defaultStart,
  defaultDue,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projectId: string;
  task?: PlanningTask;
  tasks: readonly PlanningTask[];
  defaultStart: string;
  defaultDue: string;
  onSave: (task: PlanningTask) => Promise<void>;
}) {
  const [title, setTitle] = React.useState("");
  const [wbs, setWbs] = React.useState("");
  const [owner, setOwner] = React.useState("");
  const [start, setStart] = React.useState(defaultStart);
  const [due, setDue] = React.useState(defaultDue);
  const [duration, setDuration] = React.useState(1);
  const [status, setStatus] = React.useState<TaskStatus>("notStarted");
  const [progress, setProgress] = React.useState(0);
  const [dependencies, setDependencies] = React.useState<string[]>([]);
  const [notes, setNotes] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  React.useEffect(() => {
    if (!open) return;
    setTitle(task?.title ?? "");
    setWbs(task?.wbs ?? String(tasks.length + 1));
    setOwner(task?.owner ?? "");
    setStart(task?.plannedStart ?? defaultStart);
    setDue(task?.dueDate ?? defaultDue);
    setDuration(task?.durationWorkdays ?? 1);
    setStatus(task?.status ?? "notStarted");
    setProgress(task?.percentComplete ?? 0);
    setDependencies([...(task?.dependencyIds ?? [])]);
    setNotes(task?.notes ?? "");
  }, [defaultDue, defaultStart, open, task, tasks.length]);
  async function save() {
    setBusy(true);
    try {
      const now = new Date().toISOString();
      await onSave(
        parsePlanningTask({
          schemaVersion: 1,
          id: task?.id ?? crypto.randomUUID(),
          projectId,
          wbs,
          parentTaskId: task?.parentTaskId ?? null,
          title,
          owner,
          status,
          percentComplete: progress,
          plannedStart: start,
          dueDate: due,
          durationWorkdays: duration,
          dependencyIds: dependencies,
          fixedStart: task?.fixedStart ?? null,
          linkedCapabilityId: task?.linkedCapabilityId ?? null,
          linkedMissionRequirementId: task?.linkedMissionRequirementId ?? null,
          notes: notes.trim() || null,
          sourceEvidence: task?.sourceEvidence ?? "Manual plan entry",
          isSummary: false,
          createdAt: task?.createdAt ?? now,
          updatedAt: now,
        }),
      );
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-auto">
        <DialogHeader>
          <DialogTitle>
            {task ? "Edit planning task" : "New planning task"}
          </DialogTitle>
        </DialogHeader>
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="grid gap-1 text-sm sm:col-span-2">
            Task
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setTitle(event.target.value)}
              value={title}
            />
          </label>
          <label className="grid gap-1 text-sm">
            WBS
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setWbs(event.target.value)}
              value={wbs}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Owner
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setOwner(event.target.value)}
              value={owner}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Start
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setStart(event.target.value)}
              type="date"
              value={start}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Due
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setDue(event.target.value)}
              type="date"
              value={due}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Duration (working days)
            <input
              className="rounded border bg-background px-3 py-2"
              min={1}
              onChange={(event) => setDuration(Number(event.target.value))}
              type="number"
              value={duration}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Status
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setStatus(event.target.value as TaskStatus)}
              value={status}
            >
              <option value="notStarted">Not started</option>
              <option value="inProgress">In progress</option>
              <option value="blocked">Blocked</option>
              <option value="complete">Complete</option>
              <option value="cancelled">Cancelled</option>
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            Completion
            <input
              className="rounded border bg-background px-3 py-2"
              max={100}
              min={0}
              onChange={(event) => setProgress(Number(event.target.value))}
              type="number"
              value={progress}
            />
          </label>
          <fieldset className="grid gap-1 text-sm sm:col-span-2">
            <legend>Finish-to-start dependencies</legend>
            <div className="grid max-h-32 gap-1 overflow-auto rounded border p-2">
              {tasks.filter((item) => item.id !== task?.id).length ? (
                tasks
                  .filter((item) => item.id !== task?.id && !item.isSummary)
                  .map((item) => (
                    <label className="flex items-center gap-2" key={item.id}>
                      <input
                        checked={dependencies.includes(item.id)}
                        onChange={(event) =>
                          setDependencies((current) =>
                            event.target.checked
                              ? [...current, item.id]
                              : current.filter((id) => id !== item.id),
                          )
                        }
                        type="checkbox"
                      />
                      {item.wbs} {item.title}
                    </label>
                  ))
              ) : (
                <span className="text-muted-foreground">No earlier tasks</span>
              )}
            </div>
          </fieldset>
          <label className="grid gap-1 text-sm sm:col-span-2">
            Notes
            <textarea
              className="min-h-16 rounded border bg-background px-3 py-2"
              onChange={(event) => setNotes(event.target.value)}
              value={notes}
            />
          </label>
        </div>
        <div className="flex justify-end gap-2">
          <button
            className="rounded border px-3 py-2 text-sm"
            onClick={() => onOpenChange(false)}
            type="button"
          >
            Cancel
          </button>
          <button
            className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
            disabled={
              busy ||
              !title.trim() ||
              !wbs.trim() ||
              !owner.trim() ||
              !start ||
              !due ||
              duration < 1
            }
            onClick={() => void save()}
            type="button"
          >
            Save task
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
