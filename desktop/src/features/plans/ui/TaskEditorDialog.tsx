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
import {
  defaultTaskDetails,
  parsePlanningTaskDetails,
  type PlanningTaskDetailsV1,
  type TaskExecutionMode,
  type TaskOutputType,
} from "../domain/extendedContracts";
import { COMMAND_TEAM_PERSONAS } from "@/features/command-console/domain/commandTeam";

export function TaskEditorDialog({
  open,
  onOpenChange,
  projectId,
  task,
  taskDetails,
  tasks,
  defaultStart,
  defaultDue,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projectId: string;
  task?: PlanningTask;
  taskDetails?: PlanningTaskDetailsV1;
  tasks: readonly PlanningTask[];
  defaultStart: string;
  defaultDue: string;
  onSave: (task: PlanningTask, details: PlanningTaskDetailsV1) => Promise<void>;
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
  const [department, setDepartment] = React.useState("XO");
  const [position, setPosition] = React.useState("Executive Officer");
  const [individual, setIndividual] = React.useState("");
  const [agentId, setAgentId] = React.useState("");
  const [dueTime, setDueTime] = React.useState("16:00");
  const [executionMode, setExecutionMode] =
    React.useState<TaskExecutionMode>("manual");
  const [outputType, setOutputType] =
    React.useState<TaskOutputType>("response");
  const [locked, setLocked] = React.useState(false);
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
    const details =
      taskDetails ?? (task ? defaultTaskDetails(task) : undefined);
    setDepartment(details?.department ?? "XO");
    setPosition(details?.position ?? "Executive Officer");
    setIndividual(details?.individual ?? "");
    setAgentId(details?.agentId ?? "");
    setDueTime(details?.dueTime ?? "16:00");
    setExecutionMode(details?.executionMode ?? "manual");
    setOutputType(details?.outputType ?? "response");
    setLocked(details?.locked ?? false);
  }, [defaultDue, defaultStart, open, task, taskDetails, tasks.length]);
  async function save() {
    setBusy(true);
    try {
      const now = new Date().toISOString();
      const taskId = task?.id ?? crypto.randomUUID();
      const parsedTask = parsePlanningTask({
        schemaVersion: 1,
        id: taskId,
        projectId,
        wbs,
        parentTaskId: task?.parentTaskId ?? null,
        title,
        owner: position.trim() || owner,
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
      });
      const parsedDetails = parsePlanningTaskDetails({
        schemaVersion: 1,
        id: taskDetails?.id ?? `details:${taskId}`,
        projectId,
        taskId,
        department,
        position,
        individual: individual.trim() || null,
        agentId: agentId || null,
        dueTime: dueTime || null,
        executionMode,
        outputType,
        playbookId: taskDetails?.playbookId ?? null,
        playbookRevisionId: taskDetails?.playbookRevisionId ?? null,
        locked,
        createdAt: taskDetails?.createdAt ?? task?.createdAt ?? now,
        updatedAt: now,
      });
      await onSave(parsedTask, parsedDetails);
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
            Department / HOD
            <input
              className="rounded border bg-background px-3 py-2"
              list="planning-departments"
              onChange={(event) => {
                setDepartment(event.target.value);
                if (!position.trim()) setPosition(event.target.value);
              }}
              value={department}
            />
            <datalist id="planning-departments">
              <option value="XO">Executive Officer</option>
              <option value="MEO">Marine Engineering Officer</option>
              <option value="WEEO">
                Weapons Electrical Engineering Officer
              </option>
              <option value="SO">Supply Officer</option>
              <option value="Navigation">Navigation Department</option>
              <option value="Operations">Operations Department</option>
            </datalist>
          </label>
          <label className="grid gap-1 text-sm">
            Responsible position
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => {
                setPosition(event.target.value);
                setOwner(event.target.value);
              }}
              value={position}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Specific individual (optional)
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setIndividual(event.target.value)}
              placeholder="Name or billet"
              value={individual}
            />
          </label>
          <label className="grid gap-1 text-sm">
            AI adviser (optional)
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setAgentId(event.target.value)}
              value={agentId}
            >
              <option value="">Ships company only</option>
              {COMMAND_TEAM_PERSONAS.map((persona) => (
                <option key={persona.personaId} value={persona.personaId}>
                  {persona.label}
                </option>
              ))}
            </select>
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
            Due time (ship time)
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setDueTime(event.target.value)}
              type="time"
              value={dueTime}
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
              <option value="forReview">For review</option>
              <option value="complete">Complete</option>
              <option value="cancelled">Cancelled</option>
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            AI execution
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) =>
                setExecutionMode(event.target.value as TaskExecutionMode)
              }
              value={executionMode}
            >
              <option value="manual">Manual start only</option>
              <option value="scheduled">Start one hour before due</option>
              <option value="hybrid">Manual or one hour before due</option>
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            Required output
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) =>
                setOutputType(event.target.value as TaskOutputType)
              }
              value={outputType}
            >
              <option value="response">Response in Command Adviser</option>
              <option value="docx">Word document</option>
              <option value="pptx">PowerPoint presentation</option>
              <option value="xlsx">Excel workbook</option>
              <option value="pdf">PDF</option>
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
          <label className="flex items-center gap-2 text-sm sm:col-span-2">
            <input
              checked={locked}
              onChange={(event) => setLocked(event.target.checked)}
              type="checkbox"
            />
            Lock this task against playbook rescheduling
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
              !department.trim() ||
              !position.trim() ||
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
