import * as React from "react";
import { pickBattleRhythmDocument } from "@/shared/api/tauriBattleRhythm";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  interpretPlanDocument,
  type PlanImportProposal,
} from "../domain/importProposal";
import {
  parsePlanningTask,
  type PlanningProject,
  type PlanningTask,
} from "../domain/contracts";

export function PlanImportReviewDialog({
  open,
  onOpenChange,
  project,
  existingTasks,
  onApply,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  project: PlanningProject;
  existingTasks: readonly PlanningTask[];
  onApply: (tasks: readonly PlanningTask[]) => Promise<void>;
}) {
  const [proposal, setProposal] = React.useState<PlanImportProposal>();
  const [documentName, setDocumentName] = React.useState("");
  const [documentHash, setDocumentHash] = React.useState("");
  const [selectedWbs, setSelectedWbs] = React.useState<Set<string>>(new Set());
  const [reviewedUncertainties, setReviewedUncertainties] = React.useState<
    Set<string>
  >(new Set());
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string>();

  React.useEffect(() => {
    if (open) return;
    setProposal(undefined);
    setDocumentName("");
    setDocumentHash("");
    setSelectedWbs(new Set());
    setReviewedUncertainties(new Set());
    setError(undefined);
  }, [open]);

  async function chooseDocument() {
    setBusy(true);
    setError(undefined);
    try {
      const document = await pickBattleRhythmDocument();
      if (!document) return;
      const interpreted = interpretPlanDocument(document, project);
      setProposal(interpreted);
      setDocumentName(document.filename);
      setDocumentHash(document.sha256);
      setSelectedWbs(new Set(interpreted.tasks.map((task) => task.wbs)));
      setReviewedUncertainties(new Set());
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Plan document import failed.",
      );
    } finally {
      setBusy(false);
    }
  }

  const blockingUnreviewed =
    proposal?.uncertainties.filter(
      (item) =>
        item.blocking &&
        !reviewedUncertainties.has(`${item.location}:${item.message}`),
    ) ?? [];

  async function apply() {
    if (!proposal || !documentName || !documentHash) return;
    setBusy(true);
    setError(undefined);
    try {
      const selected = proposal.tasks.filter((task) =>
        selectedWbs.has(task.wbs),
      );
      const byWbs = new Map(existingTasks.map((task) => [task.wbs, task]));
      const idByWbs = new Map(existingTasks.map((task) => [task.wbs, task.id]));
      for (const task of selected)
        if (!idByWbs.has(task.wbs)) idByWbs.set(task.wbs, crypto.randomUUID());
      const missingSelectedDependency = selected.find((task) =>
        task.dependencyWbs.some((dependency) => !idByWbs.has(dependency)),
      );
      if (missingSelectedDependency)
        throw new Error(
          `Select dependency tasks required by WBS ${missingSelectedDependency.wbs}.`,
        );
      const now = new Date().toISOString();
      const imported = selected.map((task) => {
        const existing = byWbs.get(task.wbs);
        const id = idByWbs.get(task.wbs);
        if (!id) throw new Error(`WBS ${task.wbs} has no task identity.`);
        return parsePlanningTask({
          schemaVersion: 1,
          id,
          projectId: project.id,
          wbs: task.wbs,
          parentTaskId: existing?.parentTaskId ?? null,
          title: task.title,
          owner: task.owner,
          status:
            task.percentComplete === 100
              ? "complete"
              : task.percentComplete > 0
                ? "inProgress"
                : "notStarted",
          percentComplete: task.percentComplete,
          plannedStart: task.plannedStart,
          dueDate: task.dueDate,
          durationWorkdays: task.durationWorkdays,
          dependencyIds: task.dependencyWbs.map((dependency) => {
            const dependencyId = idByWbs.get(dependency);
            if (!dependencyId)
              throw new Error(`WBS ${dependency} has no task identity.`);
            return dependencyId;
          }),
          fixedStart: existing?.fixedStart ?? null,
          linkedCapabilityId: existing?.linkedCapabilityId ?? null,
          linkedMissionRequirementId:
            existing?.linkedMissionRequirementId ?? null,
          notes: existing?.notes ?? null,
          sourceEvidence: `${documentName} · ${task.sourceLocation} · sha256:${documentHash}`,
          isSummary: false,
          createdAt: existing?.createdAt ?? now,
          updatedAt: now,
        });
      });
      await onApply(imported);
      onOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Plan import failed.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[88vh] max-w-4xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Review deployment plan import</DialogTitle>
        </DialogHeader>
        <div className="grid gap-4">
          <p className="text-sm text-muted-foreground">
            Import reviewed WBS tasks from Word, Excel, or PDF. Source rows are
            retained on each signed task.
          </p>
          <button
            className="w-fit rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
            disabled={busy}
            onClick={chooseDocument}
            type="button"
          >
            {busy ? "Reading plan…" : "Choose Word, Excel, or PDF"}
          </button>
          {proposal ? (
            <>
              <div className="rounded border p-3 text-sm">
                <strong>{documentName}</strong>
                <p className="text-xs text-muted-foreground">
                  {proposal.tasks.length} tasks proposed · deterministic WBS
                  interpretation
                </p>
              </div>
              {proposal.uncertainties.length ? (
                <section className="rounded border border-amber-500/50 bg-amber-500/10 p-3">
                  <h3 className="text-sm font-semibold">
                    Items requiring review
                  </h3>
                  <div className="mt-2 grid gap-2">
                    {proposal.uncertainties.map((item) => {
                      const key = `${item.location}:${item.message}`;
                      return (
                        <label
                          className="flex items-start gap-2 text-xs"
                          key={key}
                        >
                          <input
                            checked={reviewedUncertainties.has(key)}
                            onChange={(event) => {
                              const next = new Set(reviewedUncertainties);
                              if (event.target.checked) next.add(key);
                              else next.delete(key);
                              setReviewedUncertainties(next);
                            }}
                            type="checkbox"
                          />
                          <span>
                            <strong>{item.location}</strong>: {item.message}
                          </span>
                        </label>
                      );
                    })}
                  </div>
                </section>
              ) : null}
              <div className="grid max-h-80 gap-2 overflow-y-auto">
                {proposal.tasks.map((task) => (
                  <label
                    className="flex items-start gap-3 rounded border p-3 text-sm"
                    key={`${task.wbs}:${task.sourceLocation}`}
                  >
                    <input
                      checked={selectedWbs.has(task.wbs)}
                      onChange={(event) => {
                        const next = new Set(selectedWbs);
                        if (event.target.checked) next.add(task.wbs);
                        else next.delete(task.wbs);
                        setSelectedWbs(next);
                      }}
                      type="checkbox"
                    />
                    <span className="min-w-0">
                      <strong>
                        {task.wbs} {task.title}
                      </strong>
                      <span className="block text-xs text-muted-foreground">
                        {task.owner} · {task.plannedStart} to {task.dueDate} ·{" "}
                        {task.durationWorkdays} working days
                        {task.dependencyWbs.length
                          ? ` · after ${task.dependencyWbs.join(", ")}`
                          : ""}
                      </span>
                      <span className="block truncate text-2xs text-muted-foreground">
                        {task.sourceLocation}
                      </span>
                    </span>
                  </label>
                ))}
              </div>
            </>
          ) : null}
          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
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
                !proposal ||
                selectedWbs.size === 0 ||
                blockingUnreviewed.length > 0
              }
              onClick={apply}
              type="button"
            >
              Import reviewed tasks
            </button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
