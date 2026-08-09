import * as React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  parseMissionConstraint,
  type ConstraintSeverity,
  type ConstraintStatus,
  type ConstraintType,
  type MissionConstraint,
  type PlanningTask,
} from "../domain/contracts";

export function ConstraintEditorDialog({
  open,
  onOpenChange,
  projectId,
  constraint,
  tasks,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projectId: string;
  constraint?: MissionConstraint;
  tasks: readonly PlanningTask[];
  onSave: (constraint: MissionConstraint) => Promise<void>;
}) {
  const [description, setDescription] = React.useState("");
  const [owner, setOwner] = React.useState("");
  const [type, setType] = React.useState<ConstraintType>("defect");
  const [severity, setSeverity] = React.useState<ConstraintSeverity>("medium");
  const [status, setStatus] = React.useState<ConstraintStatus>("open");
  const [taskId, setTaskId] = React.useState("");
  const [missionRequirement, setMissionRequirement] = React.useState("");
  const [requiredDate, setRequiredDate] = React.useState("");
  const [disposition, setDisposition] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  React.useEffect(() => {
    if (!open) return;
    setDescription(constraint?.description ?? "");
    setOwner(constraint?.owner ?? "");
    setType(constraint?.type ?? "defect");
    setSeverity(constraint?.severity ?? "medium");
    setStatus(constraint?.status ?? "open");
    setTaskId(constraint?.linkedTaskId ?? "");
    setMissionRequirement(constraint?.linkedMissionRequirementId ?? "");
    setRequiredDate(constraint?.requiredDate ?? "");
    setDisposition(constraint?.dispositionNote ?? "");
  }, [constraint, open]);
  const candidate = status === "oplimCandidate" || status === "riskCandidate";
  async function save() {
    setBusy(true);
    try {
      const now = new Date().toISOString();
      await onSave(
        parseMissionConstraint({
          schemaVersion: 1,
          id: constraint?.id ?? crypto.randomUUID(),
          projectId,
          type,
          description,
          owner,
          severity,
          status,
          linkedMissionRequirementId: missionRequirement.trim() || null,
          linkedCapabilityId: constraint?.linkedCapabilityId ?? null,
          linkedTaskId: taskId || null,
          linkedMilestoneId: constraint?.linkedMilestoneId ?? null,
          requiredDate: requiredDate || null,
          dispositionNote: disposition.trim() || null,
          sourceEvidence: constraint?.sourceEvidence ?? "Manual plan entry",
          createdAt: constraint?.createdAt ?? now,
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
            {constraint
              ? "Update mission constraint"
              : "New mission constraint"}
          </DialogTitle>
        </DialogHeader>
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="grid gap-1 text-sm sm:col-span-2">
            Constraint and operational effect
            <textarea
              className="min-h-20 rounded border bg-background px-3 py-2"
              onChange={(event) => setDescription(event.target.value)}
              value={description}
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
            Type
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) =>
                setType(event.target.value as ConstraintType)
              }
              value={type}
            >
              <option value="defect">Defect</option>
              <option value="missingCapability">Missing capability</option>
              <option value="readiness">Readiness</option>
              <option value="externalDependency">External dependency</option>
              <option value="assumption">Assumption</option>
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            Severity
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) =>
                setSeverity(event.target.value as ConstraintSeverity)
              }
              value={severity}
            >
              <option value="low">Low</option>
              <option value="medium">Medium</option>
              <option value="high">High</option>
              <option value="critical">Critical</option>
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            Disposition
            <select
              aria-label="Disposition"
              className="rounded border bg-background px-3 py-2"
              onChange={(event) =>
                setStatus(event.target.value as ConstraintStatus)
              }
              value={status}
            >
              <option value="open">Open</option>
              <option value="mitigated">Mitigated</option>
              <option value="resolved">Resolved</option>
              <option value="missionChanged">Mission changed</option>
              <option value="oplimCandidate">OPLIM candidate</option>
              <option value="riskCandidate">Risk candidate</option>
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            Linked task
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setTaskId(event.target.value)}
              value={taskId}
            >
              <option value="">No linked task</option>
              {tasks.map((task) => (
                <option key={task.id} value={task.id}>
                  {task.wbs} {task.title}
                </option>
              ))}
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            Mission requirement
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setMissionRequirement(event.target.value)}
              placeholder="e.g. Conduct seaboat operations"
              value={missionRequirement}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Required resolution
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setRequiredDate(event.target.value)}
              type="date"
              value={requiredDate}
            />
          </label>
          <label className="grid gap-1 text-sm sm:col-span-2">
            Mitigation or command disposition
            <textarea
              className="min-h-16 rounded border bg-background px-3 py-2"
              onChange={(event) => setDisposition(event.target.value)}
              value={disposition}
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
              !description.trim() ||
              !owner.trim() ||
              (!taskId && !missionRequirement.trim()) ||
              (candidate && !disposition.trim())
            }
            onClick={() => void save()}
            type="button"
          >
            Save constraint
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
