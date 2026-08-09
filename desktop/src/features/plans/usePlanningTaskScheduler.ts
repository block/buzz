import * as React from "react";
import { useBattleRhythmQuery } from "@/features/battle-rhythm/hooks";
import { getYearRange } from "@/features/battle-rhythm/domain/dateRange";
import {
  deriveShipRoutinePeriods,
  shipStateAt,
} from "@/features/battle-rhythm/domain/shipRoutine";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  executePlanningTask,
  generateTaskArtifact,
} from "@/shared/api/tauriProjectExecution";
import { usePlanMutations, usePlansQuery } from "./hooks";
import { dueAutomaticTasks } from "./domain/taskDue";
import type { PlanningTaskExecutionV1 } from "./domain/extendedContracts";

export function PlanningTaskScheduler() {
  const identity = useIdentityQuery();
  const pubkey = identity.data?.pubkey;
  const plans = usePlansQuery(pubkey);
  const mutations = usePlanMutations(pubkey ?? "");
  const publishExecution = mutations.execution.mutateAsync;
  const publishArtifact = mutations.artifact.mutateAsync;
  const publishTask = mutations.task.mutateAsync;
  const day = new Date().toISOString().slice(0, 10);
  const range = React.useMemo(
    () => getYearRange(day, "Australia/Sydney", 24),
    [day],
  );
  const rhythm = useBattleRhythmQuery(pubkey, range);
  const periods = React.useMemo(
    () =>
      deriveShipRoutinePeriods(
        rhythm.data?.sources ?? [],
        rhythm.data?.events ?? [],
        range,
      ),
    [range, rhythm.data?.events, rhythm.data?.sources],
  );
  const running = React.useRef(false);
  React.useEffect(() => {
    if (!plans.data || !pubkey) return;
    const tick = async () => {
      if (running.current) return;
      running.current = true;
      try {
        const due = dueAutomaticTasks({
          tasks: plans.data.tasks,
          details: plans.data.details,
          executions: plans.data.executions,
          now: new Date().toISOString(),
          timeZoneFor: (date) =>
            shipStateAt(periods, `${date}T12:00:00Z`).timeZone,
        });
        for (const item of due) {
          const project = plans.data.projects.find(
            (candidate) => candidate.id === item.task.projectId,
          );
          if (!project) continue;
          const startedAt = new Date().toISOString();
          const queued: PlanningTaskExecutionV1 = {
            schemaVersion: 1,
            id: item.timing.claimKey,
            projectId: project.id,
            taskId: item.task.id,
            status: "queued",
            mode: item.details.executionMode,
            summary: null,
            body: null,
            missingInputs: [],
            assumptions: [],
            provider: null,
            model: null,
            startedAt,
            completedAt: null,
            error: null,
            lateStart: item.lateStart,
          };
          await publishExecution(queued);
          try {
            const dependencies = item.task.dependencyIds.map((id) => {
              const task = plans.data?.tasks.find(
                (candidate) => candidate.id === id,
              );
              const execution = plans.data?.executions
                .filter((candidate) => candidate.taskId === id)
                .sort((left, right) =>
                  right.startedAt.localeCompare(left.startedAt),
                )[0];
              return {
                title: task?.title ?? id,
                status: task?.status ?? "missing",
                summary: execution?.summary ?? null,
              };
            });
            const result = await executePlanningTask({
              taskTitle: item.task.title,
              instructions:
                item.task.notes ??
                `Complete ${item.task.title} for ${project.title}.`,
              adviserId: item.details.agentId,
              outputType: item.details.outputType,
              dependencies,
              planningContext: {
                project,
                task: item.task,
                assignment: item.details,
              },
            });
            const completedAt = new Date().toISOString();
            await publishExecution({
              ...queued,
              status: "forReview",
              summary: result.summary,
              body: result.body,
              missingInputs: result.missingInputs,
              assumptions: result.assumptions,
              provider: result.provider,
              model: result.model,
              completedAt,
            });
            if (result.outputType !== "response") {
              const generated = await generateTaskArtifact({
                projectTitle: project.title,
                taskTitle: item.task.title,
                format: result.outputType,
                title: result.summary,
                body: result.body,
              });
              await publishArtifact({
                schemaVersion: 1,
                id: crypto.randomUUID(),
                projectId: project.id,
                taskId: item.task.id,
                executionId: queued.id,
                fileName: generated.fileName,
                path: generated.path,
                format: generated.format,
                storageState: generated.storageState,
                agentId: item.details.agentId,
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
            await publishTask({
              ...item.task,
              status: "forReview",
              percentComplete: Math.max(90, item.task.percentComplete),
              updatedAt: completedAt,
            });
          } catch (cause) {
            await publishExecution({
              ...queued,
              status: "failed",
              completedAt: new Date().toISOString(),
              error:
                cause instanceof Error ? cause.message : "Execution failed.",
            });
          }
        }
      } finally {
        running.current = false;
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 60_000);
    const visible = () => {
      if (document.visibilityState === "visible") void tick();
    };
    document.addEventListener("visibilitychange", visible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [
    periods,
    plans.data,
    pubkey,
    publishArtifact,
    publishExecution,
    publishTask,
  ]);
  return null;
}
