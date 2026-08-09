import * as React from "react";
import { FileOutput, Play } from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  executePlanningTask,
  generateTaskArtifact,
} from "@/shared/api/tauriProjectExecution";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { PlanningProject, PlanningTask } from "../domain/contracts";
import type {
  PlanningTaskArtifactV1,
  PlanningTaskDetailsV1,
  PlanningTaskExecutionV1,
} from "../domain/extendedContracts";

export function TaskExecutionPanel({
  open,
  onOpenChange,
  project,
  task,
  details,
  tasks,
  executions,
  artifacts,
  onSaveExecution,
  onSaveArtifact,
  onMoveForReview,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  project: PlanningProject;
  task: PlanningTask;
  details: PlanningTaskDetailsV1;
  tasks: readonly PlanningTask[];
  executions: readonly PlanningTaskExecutionV1[];
  artifacts: readonly PlanningTaskArtifactV1[];
  onSaveExecution: (execution: PlanningTaskExecutionV1) => Promise<void>;
  onSaveArtifact: (artifact: PlanningTaskArtifactV1) => Promise<void>;
  onMoveForReview: (task: PlanningTask) => Promise<void>;
}) {
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string>();
  const taskExecutions = executions
    .filter((item) => item.taskId === task.id)
    .sort((left, right) => right.startedAt.localeCompare(left.startedAt));
  const latest = taskExecutions[0];
  const taskArtifacts = artifacts.filter((item) => item.taskId === task.id);
  async function run() {
    setBusy(true);
    setError(undefined);
    const id = crypto.randomUUID();
    const startedAt = new Date().toISOString();
    try {
      const dependencies = task.dependencyIds.map((dependencyId) => {
        const dependency = tasks.find((item) => item.id === dependencyId);
        const prior = executions
          .filter((item) => item.taskId === dependencyId)
          .sort((left, right) =>
            right.startedAt.localeCompare(left.startedAt),
          )[0];
        return {
          title: dependency?.title ?? dependencyId,
          status: dependency?.status ?? "missing",
          summary: prior?.summary ?? null,
        };
      });
      const result = await executePlanningTask({
        taskTitle: task.title,
        instructions:
          task.notes ??
          `Complete ${task.title} for ${project.title}. The required date is ${task.dueDate ?? "not set"}.`,
        adviserId: details.agentId,
        outputType: details.outputType,
        dependencies,
        planningContext: {
          project: {
            title: project.title,
            purpose: project.purpose,
            missionReadyDate: project.missionReadyDate,
            assumptions: project.assumptions,
          },
          task: {
            wbs: task.wbs,
            department: details.department,
            position: details.position,
            dueDate: task.dueDate,
            dueTime: details.dueTime,
          },
        },
      });
      const completedAt = new Date().toISOString();
      const execution: PlanningTaskExecutionV1 = {
        schemaVersion: 1,
        id,
        projectId: project.id,
        taskId: task.id,
        status: "forReview",
        mode: details.executionMode,
        summary: result.summary,
        body: result.body,
        missingInputs: result.missingInputs,
        assumptions: result.assumptions,
        provider: result.provider,
        model: result.model,
        startedAt,
        completedAt,
        error: null,
        lateStart: false,
      };
      await onSaveExecution(execution);
      if (result.outputType !== "response") {
        const generated = await generateTaskArtifact({
          projectTitle: project.title,
          taskTitle: task.title,
          format: result.outputType,
          title: result.summary,
          body: result.body,
        });
        await onSaveArtifact({
          schemaVersion: 1,
          id: crypto.randomUUID(),
          projectId: project.id,
          taskId: task.id,
          executionId: id,
          fileName: generated.fileName,
          path: generated.path,
          format: generated.format,
          storageState: generated.storageState,
          agentId: details.agentId,
          provider: result.provider,
          model: result.model,
          summary: result.summary,
          missingInputWarning: result.missingInputs.length
            ? result.missingInputs.join("; ")
            : null,
          sha256: generated.sha256,
          sizeBytes: generated.sizeBytes,
          createdAt: completedAt,
        });
      }
      await onMoveForReview({
        ...task,
        status: "forReview",
        percentComplete: Math.max(90, task.percentComplete),
        updatedAt: completedAt,
      });
    } catch (cause) {
      const message =
        cause instanceof Error ? cause.message : "Task execution failed.";
      setError(message);
      try {
        await onSaveExecution({
          schemaVersion: 1,
          id,
          projectId: project.id,
          taskId: task.id,
          status: "failed",
          mode: details.executionMode,
          summary: null,
          body: null,
          missingInputs: [],
          assumptions: [],
          provider: null,
          model: null,
          startedAt,
          completedAt: new Date().toISOString(),
          error: message,
          lateStart: false,
        });
      } catch {
        // The visible execution error remains authoritative when relay logging
        // is also unavailable.
      }
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[88vh] max-w-3xl overflow-auto">
        <DialogHeader>
          <DialogTitle>AI task — {task.title}</DialogTitle>
        </DialogHeader>
        <div className="grid gap-3 text-sm">
          <div className="rounded border bg-muted/30 p-3">
            <p>
              Assigned: <strong>{details.position}</strong>
              {details.agentId ? " with Command Team adviser" : ""}
            </p>
            <p>
              Mode: {details.executionMode} · Output: {details.outputType}
            </p>
            <p>
              Due: {task.dueDate ?? "not set"} {details.dueTime ?? "16:00"} ship
              time
            </p>
          </div>
          <button
            className="w-fit rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
            disabled={busy || !details.agentId}
            onClick={() => void run()}
            type="button"
          >
            <Play className="mr-1 inline h-4 w-4" />
            {busy ? "Adviser working…" : "Run now"}
          </button>
          {!details.agentId ? (
            <p className="text-sm text-muted-foreground">
              Assign a Command Team adviser in Edit task before running it.
            </p>
          ) : null}
          {error ? (
            <p className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-800 dark:text-red-200">
              {error}
            </p>
          ) : null}
          {latest ? (
            <section className="rounded-lg border p-4">
              <h3 className="font-semibold">Latest adviser output</h3>
              <p className="mt-2 font-medium">
                {latest.summary ?? latest.error ?? latest.status}
              </p>
              {latest.body ? (
                <p className="mt-2 whitespace-pre-wrap">{latest.body}</p>
              ) : null}
              {latest.missingInputs.length ? (
                <div className="mt-3 rounded border border-amber-500/40 bg-amber-500/10 p-3">
                  <p className="font-medium">
                    Missing inputs — check before use
                  </p>
                  <ul className="mt-1 list-disc pl-5">
                    {latest.missingInputs.map((item) => (
                      <li key={item}>{item}</li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <p className="mt-2 text-xs text-muted-foreground">
                Provider: {latest.provider ?? "not recorded"} · Review required
                before the task is marked complete.
              </p>
            </section>
          ) : null}
          {taskArtifacts.length ? (
            <section className="rounded-lg border p-4">
              <h3 className="font-semibold">Output files</h3>
              <div className="mt-2 grid gap-2">
                {taskArtifacts.map((artifact) => (
                  <button
                    className="flex items-center gap-2 rounded border p-2 text-left text-sm"
                    key={artifact.id}
                    onClick={() => void openPath(artifact.path)}
                    type="button"
                  >
                    <FileOutput className="h-4 w-4" />
                    <span>
                      <span className="block font-medium">
                        {artifact.fileName}
                      </span>
                      <span className="block break-all text-xs text-muted-foreground">
                        {artifact.path}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            </section>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
