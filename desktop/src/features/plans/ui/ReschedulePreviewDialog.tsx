import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { PlanningSchedule } from "@/shared/api/tauriPlans";
import type { PlanningTask } from "../domain/contracts";

function affected(
  before: PlanningSchedule,
  after: PlanningSchedule,
  tasks: readonly PlanningTask[],
) {
  const prior = new Map(before.tasks.map((item) => [item.taskId, item]));
  return after.tasks
    .filter((item) => {
      const value = prior.get(item.taskId);
      return (
        !value ||
        value.earliestStart !== item.earliestStart ||
        value.earliestFinish !== item.earliestFinish ||
        value.critical !== item.critical
      );
    })
    .map((item) => ({
      ...item,
      title:
        tasks.find((task) => task.id === item.taskId)?.title ?? item.taskId,
    }));
}

export function ReschedulePreviewDialog({
  open,
  onOpenChange,
  task,
  tasks,
  before,
  after,
  onApply,
  busy,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  task?: PlanningTask;
  tasks: readonly PlanningTask[];
  before?: PlanningSchedule;
  after?: PlanningSchedule;
  onApply: () => Promise<void>;
  busy: boolean;
}) {
  const changes = before && after && task ? affected(before, after, tasks) : [];
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Review schedule change</DialogTitle>
        </DialogHeader>
        {task && after ? (
          <div className="grid gap-3 text-sm">
            <p>
              <strong>{task.title}</strong> will move to {task.plannedStart}.
              Nothing changes until you apply this preview.
            </p>
            <div className="rounded border p-3">
              <p className="font-medium">
                {changes.length} schedule item
                {changes.length === 1 ? "" : "s"} affected
              </p>
              <ul className="mt-2 grid gap-1">
                {changes.map((item) => (
                  <li key={item.taskId}>
                    {item.title}: {item.earliestStart}–{item.earliestFinish}
                    {item.critical ? " · critical path" : ""}
                  </li>
                ))}
              </ul>
            </div>
            <p
              className={
                after.missionReadyAtRisk
                  ? "rounded border border-red-500/40 bg-red-500/10 p-3"
                  : "rounded border border-emerald-500/40 bg-emerald-500/10 p-3"
              }
            >
              {after.missionReadyAtRisk
                ? "Mission-ready date would be at risk."
                : "Mission-ready date remains on track."}
            </p>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            Calculating schedule impact…
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button
            className="rounded border px-3 py-2 text-sm"
            disabled={busy}
            onClick={() => onOpenChange(false)}
            type="button"
          >
            Cancel
          </button>
          <button
            className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
            disabled={busy || !task || !after}
            onClick={() => void onApply()}
            type="button"
          >
            {busy ? "Applying…" : "Apply schedule change"}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
