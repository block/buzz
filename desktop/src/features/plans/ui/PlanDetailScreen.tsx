import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ClipboardCheck, FileUp, Plus } from "lucide-react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useBattleRhythmQuery } from "@/features/battle-rhythm/hooks";
import { getYearRange } from "@/features/battle-rhythm/domain/dateRange";
import {
  deriveShipRoutinePeriods,
  shipStateAt,
} from "@/features/battle-rhythm/domain/shipRoutine";
import {
  calculatePlanSchedule,
  type PlanningSchedule,
} from "@/shared/api/tauriPlans";
import { usePlanMutations, usePlansQuery } from "../hooks";
import type { MissionConstraint, PlanningTask } from "../domain/contracts";
import {
  defaultTaskDetails,
  type PlanningTaskDetailsV1,
} from "../domain/extendedContracts";
import { buildHodSyncPack } from "../domain/hodSyncPack";
import { ConstraintEditorDialog } from "./ConstraintEditorDialog";
import { GanttChart } from "./GanttChart";
import { HodSyncPackDialog } from "./HodSyncPackDialog";
import { KanbanBoard, moveTaskToColumn } from "./KanbanBoard";
import type { KanbanColumnId } from "../domain/kanban";
import type { PlanningPlaybookV1 } from "../domain/extendedContracts";
import type { ScheduledPlaybookTask } from "../domain/playbookSchedule";
import { requestTaskMove } from "../domain/taskReschedule";
import { MissionConstraintsPanel } from "./MissionConstraintsPanel";
import { PlanImportReviewDialog } from "./PlanImportReviewDialog";
import { PlaybookWorkspace } from "./PlaybookWorkspace";
import { ReschedulePreviewDialog } from "./ReschedulePreviewDialog";
import { TaskEditorDialog } from "./TaskEditorDialog";
import { TaskExecutionPanel } from "./TaskExecutionPanel";
import { TaskTable } from "./TaskTable";

function today() {
  return new Date().toISOString().slice(0, 10);
}

export function PlanDetailScreen({
  planId,
  selectedTaskId,
}: {
  planId: string;
  selectedTaskId?: string;
}) {
  const identity = useIdentityQuery();
  const plans = usePlansQuery(identity.data?.pubkey);
  const mutations = usePlanMutations(identity.data?.pubkey ?? "");
  const { goPlans } = useAppNavigation();
  const project = plans.data?.projects.find((item) => item.id === planId);
  const tasks = React.useMemo(
    () => plans.data?.tasks.filter((task) => task.projectId === planId) ?? [],
    [planId, plans.data?.tasks],
  );
  const constraints = React.useMemo(
    () =>
      plans.data?.constraints.filter(
        (constraint) => constraint.projectId === planId,
      ) ?? [],
    [planId, plans.data?.constraints],
  );
  const rhythmRange = React.useMemo(
    () =>
      getYearRange(
        project?.missionReadyDate ?? today(),
        "Australia/Sydney",
        24,
      ),
    [project?.missionReadyDate],
  );
  const rhythm = useBattleRhythmQuery(identity.data?.pubkey, rhythmRange);
  const routinePeriods = React.useMemo(
    () =>
      deriveShipRoutinePeriods(
        rhythm.data?.sources ?? [],
        rhythm.data?.events ?? [],
        rhythmRange,
      ),
    [rhythm.data?.events, rhythm.data?.sources, rhythmRange],
  );
  const details = React.useMemo(
    () => plans.data?.details.filter((item) => item.projectId === planId) ?? [],
    [planId, plans.data?.details],
  );
  const schedule = useQuery({
    enabled: Boolean(project && tasks.length),
    queryKey: ["plan-schedule", project, tasks],
    queryFn: () => {
      if (!project) {
        throw new Error("Project unavailable.");
      }
      return calculatePlanSchedule({
        project,
        tasks,
        workingCalendar: {
          workingWeekdays: [1, 2, 3, 4, 5],
          excludedDates: [],
        },
        today: today(),
      });
    },
  });
  const [taskOpen, setTaskOpen] = React.useState(false);
  const [editingTask, setEditingTask] = React.useState<PlanningTask>();
  const [constraintOpen, setConstraintOpen] = React.useState(false);
  const [importOpen, setImportOpen] = React.useState(false);
  const [syncPackOpen, setSyncPackOpen] = React.useState(false);
  const [executionTask, setExecutionTask] = React.useState<PlanningTask>();
  const [editingConstraint, setEditingConstraint] =
    React.useState<MissionConstraint>();
  const [validationError, setValidationError] = React.useState<string>();
  const [moveBusy, setMoveBusy] = React.useState(false);
  const [moveProposal, setMoveProposal] = React.useState<{
    task: PlanningTask;
    before: PlanningSchedule;
    after: PlanningSchedule;
  }>();
  const [workView, setWorkView] = React.useState<
    "board" | "breakdown" | "playbooks"
  >("board");
  React.useEffect(() => {
    if (!selectedTaskId || !tasks.length) return;
    const selected = tasks.find((task) => task.id === selectedTaskId);
    if (selected) {
      setEditingTask(selected);
      setTaskOpen(true);
    }
  }, [selectedTaskId, tasks]);
  async function saveTask(
    task: PlanningTask,
    taskDetails: PlanningTaskDetailsV1,
  ) {
    if (!project) return;
    const prospective = [...tasks.filter((item) => item.id !== task.id), task];
    setValidationError(undefined);
    try {
      await calculatePlanSchedule({
        project,
        tasks: prospective,
        workingCalendar: {
          workingWeekdays: [1, 2, 3, 4, 5],
          excludedDates: [],
        },
        today: today(),
      });
    } catch (cause) {
      setValidationError(
        cause instanceof Error ? cause.message : "Task network is invalid.",
      );
      throw cause;
    }
    await mutations.task.mutateAsync(task);
    await mutations.details.mutateAsync(taskDetails);
  }
  async function moveTask(task: PlanningTask, column: KanbanColumnId) {
    setValidationError(undefined);
    try {
      await mutations.task.mutateAsync(moveTaskToColumn(task, column));
    } catch (cause) {
      setValidationError(
        cause instanceof Error
          ? `Task move was not saved: ${cause.message}`
          : "Task move was not saved. The board has been restored.",
      );
      await plans.refetch();
    }
  }
  async function previewTaskMove(task: PlanningTask, targetDate: string) {
    if (!project || !schedule.data) return;
    setValidationError(undefined);
    try {
      const locked =
        details.find((item) => item.taskId === task.id)?.locked ?? false;
      const moved = requestTaskMove(task, targetDate, locked);
      const prospective = tasks.map((item) =>
        item.id === moved.id ? moved : item,
      );
      const after = await calculatePlanSchedule({
        project,
        tasks: prospective,
        workingCalendar: {
          workingWeekdays: [1, 2, 3, 4, 5],
          excludedDates: [],
        },
        today: today(),
      });
      setMoveProposal({ task: moved, before: schedule.data, after });
    } catch (cause) {
      setValidationError(
        cause instanceof Error
          ? cause.message
          : "The proposed schedule move is invalid.",
      );
    }
  }
  async function applyTaskMove() {
    if (!moveProposal) return;
    setMoveBusy(true);
    try {
      await mutations.task.mutateAsync(moveProposal.task);
      setMoveProposal(undefined);
    } catch (cause) {
      setValidationError(
        cause instanceof Error
          ? `Schedule change was not saved: ${cause.message}`
          : "Schedule change was not saved.",
      );
    } finally {
      setMoveBusy(false);
    }
  }
  async function applyPlaybook(
    playbook: PlanningPlaybookV1,
    scheduled: readonly ScheduledPlaybookTask[],
  ) {
    if (!project) return;
    const now = new Date().toISOString();
    const taskIds = new Map<string, string>(
      scheduled.map((item) => [item.template.id, crypto.randomUUID()]),
    );
    for (const [index, item] of scheduled.entries()) {
      const taskId = taskIds.get(item.template.id);
      if (!taskId) continue;
      const task: PlanningTask = {
        schemaVersion: 1,
        id: taskId,
        projectId: project.id,
        wbs: `PB.${tasks.length + index + 1}`,
        parentTaskId: null,
        title: item.template.title,
        owner: item.template.position,
        status: "notStarted",
        percentComplete: 0,
        plannedStart: item.plannedStart,
        dueDate: item.dueDate,
        durationWorkdays: Math.max(
          1,
          Math.ceil(item.template.durationMinutes / 480),
        ),
        dependencyIds: item.dependencyIds
          .map((id) => taskIds.get(id))
          .filter((id): id is string => id !== undefined),
        fixedStart: null,
        linkedCapabilityId: item.template.linkedCapabilityId,
        linkedMissionRequirementId: item.template.linkedMissionRequirementId,
        notes: item.template.instructions,
        sourceEvidence: `Playbook ${playbook.title} revision ${playbook.revisionId}`,
        isSummary: false,
        createdAt: now,
        updatedAt: now,
      };
      const taskDetails: PlanningTaskDetailsV1 = {
        schemaVersion: 1,
        id: `details:${taskId}`,
        projectId: project.id,
        taskId,
        department: item.template.department,
        position: item.template.position,
        individual: null,
        agentId: item.template.agentId,
        dueTime: item.dueTime,
        executionMode: item.template.agentId ? "hybrid" : "manual",
        outputType: item.template.outputType,
        playbookId: playbook.id,
        playbookRevisionId: playbook.revisionId,
        locked: item.template.locked,
        createdAt: now,
        updatedAt: now,
      };
      await mutations.task.mutateAsync(task);
      await mutations.details.mutateAsync(taskDetails);
    }
  }
  async function importTasks(imported: readonly PlanningTask[]) {
    if (!project) return;
    const importedIds = new Set(imported.map((task) => task.id));
    const prospective = [
      ...tasks.filter((task) => !importedIds.has(task.id)),
      ...imported,
    ];
    setValidationError(undefined);
    try {
      await calculatePlanSchedule({
        project,
        tasks: prospective,
        workingCalendar: {
          workingWeekdays: [1, 2, 3, 4, 5],
          excludedDates: [],
        },
        today: today(),
      });
      for (const task of imported) await mutations.task.mutateAsync(task);
    } catch (cause) {
      setValidationError(
        cause instanceof Error
          ? cause.message
          : "Imported task network is invalid.",
      );
      throw cause;
    }
  }
  if (plans.isLoading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading plan…</p>;
  }
  if (!project) {
    return (
      <main className="p-6">
        <h1 className="text-xl font-semibold">Plan unavailable</h1>
        <button
          className="mt-4 rounded border px-3 py-2 text-sm"
          onClick={() => void goPlans()}
          type="button"
        >
          Return to Plans
        </button>
      </main>
    );
  }
  const scheduleValue: PlanningSchedule | undefined = schedule.data;
  const syncPack = buildHodSyncPack(
    project,
    tasks,
    details,
    scheduleValue?.tasks ?? [],
    new Date().toISOString(),
  );
  return (
    <main
      className="min-h-0 flex-1 overflow-auto p-6"
      data-testid="plan-detail-screen"
    >
      <div className="mx-auto grid max-w-7xl gap-5">
        <header>
          <button
            className="text-sm text-muted-foreground hover:text-foreground"
            onClick={() => void goPlans()}
            type="button"
          >
            <ArrowLeft className="mr-1 inline h-4 w-4" />
            Plans
          </button>
          <div className="mt-3 flex flex-wrap items-start justify-between gap-4">
            <div>
              <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
                {project.owner}
              </p>
              <h1 className="text-2xl font-semibold">{project.title}</h1>
              <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
                {project.purpose}
              </p>
            </div>
            <div className="flex gap-2">
              <button
                className="rounded border px-3 py-2 text-sm"
                onClick={() => setSyncPackOpen(true)}
                type="button"
              >
                <ClipboardCheck className="mr-1 inline h-4 w-4" />
                HOD Sync Pack
              </button>
              <button
                className="rounded border px-3 py-2 text-sm"
                onClick={() => setImportOpen(true)}
                type="button"
              >
                <FileUp className="mr-1 inline h-4 w-4" />
                Import Plan
              </button>
              <button
                className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground"
                onClick={() => {
                  setEditingTask(undefined);
                  setTaskOpen(true);
                }}
                type="button"
              >
                <Plus className="mr-1 inline h-4 w-4" />
                Add task
              </button>
            </div>
          </div>
          <div className="mt-4 flex flex-wrap gap-4 rounded-lg border bg-card p-3 text-sm">
            <span>
              Mission ready: <strong>{project.missionReadyDate}</strong>
            </span>
            <span>
              Progress: <strong>{project.progressPercent}%</strong>
            </span>
            <span>
              Tasks: <strong>{tasks.length}</strong>
            </span>
            <span>
              Open constraints:{" "}
              <strong>
                {
                  constraints.filter(
                    (constraint) =>
                      constraint.status !== "resolved" &&
                      constraint.status !== "missionChanged",
                  ).length
                }
              </strong>
            </span>
          </div>
        </header>
        {validationError ? (
          <p className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-800 dark:text-red-200">
            {validationError}
          </p>
        ) : null}
        <MissionConstraintsPanel
          constraints={constraints}
          onCreate={() => {
            setEditingConstraint(undefined);
            setConstraintOpen(true);
          }}
          onEdit={(constraint) => {
            setEditingConstraint(constraint);
            setConstraintOpen(true);
          }}
          schedule={scheduleValue?.tasks ?? []}
          tasks={tasks}
        />
        {scheduleValue ? (
          <GanttChart
            lockedTaskIds={
              new Set(
                details
                  .filter((item) => item.locked)
                  .map((item) => item.taskId),
              )
            }
            onRequestMove={(task, targetDate) =>
              void previewTaskMove(task, targetDate)
            }
            project={project}
            schedule={scheduleValue}
            tasks={tasks}
          />
        ) : tasks.length ? (
          <p className="rounded border p-4 text-sm text-muted-foreground">
            {schedule.isError
              ? "Critical path unavailable until the task network is corrected."
              : "Calculating critical path…"}
          </p>
        ) : (
          <p className="rounded border border-dashed p-8 text-center text-sm text-muted-foreground">
            Add the first task to begin the work breakdown and critical path.
          </p>
        )}
        <section className="grid gap-3">
          <div className="mb-2 flex items-center justify-between">
            <h2 className="text-base font-semibold">Project execution</h2>
            <fieldset className="flex rounded-lg border p-1">
              <legend className="sr-only">Project execution view</legend>
              <button
                aria-pressed={workView === "board"}
                className={`rounded px-3 py-1 text-sm ${
                  workView === "board"
                    ? "bg-primary text-primary-foreground"
                    : ""
                }`}
                onClick={() => setWorkView("board")}
                type="button"
              >
                Board
              </button>
              <button
                aria-pressed={workView === "breakdown"}
                className={`rounded px-3 py-1 text-sm ${
                  workView === "breakdown"
                    ? "bg-primary text-primary-foreground"
                    : ""
                }`}
                onClick={() => setWorkView("breakdown")}
                type="button"
              >
                Work breakdown
              </button>
              <button
                aria-pressed={workView === "playbooks"}
                className={`rounded px-3 py-1 text-sm ${
                  workView === "playbooks"
                    ? "bg-primary text-primary-foreground"
                    : ""
                }`}
                onClick={() => setWorkView("playbooks")}
                type="button"
              >
                Playbooks
              </button>
            </fieldset>
          </div>
          {workView === "playbooks" ? (
            <PlaybookWorkspace
              onApply={applyPlaybook}
              onSave={(playbook) =>
                mutations.playbook.mutateAsync(playbook).then(() => undefined)
              }
              playbooks={plans.data?.playbooks ?? []}
              routineAt={(date) => {
                const state = shipStateAt(routinePeriods, `${date}T12:00:00Z`);
                return {
                  routine: state.routine,
                  timeZone: state.timeZone,
                };
              }}
            />
          ) : workView === "board" ? (
            <KanbanBoard
              details={details}
              onEdit={(task) => {
                setEditingTask(task);
                setTaskOpen(true);
              }}
              onMove={moveTask}
              onRun={setExecutionTask}
              tasks={tasks}
            />
          ) : (
            <TaskTable
              details={details}
              onEdit={(task) => {
                setEditingTask(task);
                setTaskOpen(true);
              }}
              schedule={scheduleValue?.tasks ?? []}
              tasks={tasks}
            />
          )}
        </section>
      </div>
      <TaskEditorDialog
        defaultDue={project.missionReadyDate}
        defaultStart={today()}
        onOpenChange={setTaskOpen}
        onSave={saveTask}
        open={taskOpen}
        projectId={project.id}
        task={editingTask}
        taskDetails={details.find((item) => item.taskId === editingTask?.id)}
        tasks={tasks}
      />
      <ConstraintEditorDialog
        constraint={editingConstraint}
        onOpenChange={setConstraintOpen}
        onSave={(constraint) =>
          mutations.constraint.mutateAsync(constraint).then(() => undefined)
        }
        open={constraintOpen}
        projectId={project.id}
        tasks={tasks}
      />
      <PlanImportReviewDialog
        existingTasks={tasks}
        onApply={importTasks}
        onOpenChange={setImportOpen}
        open={importOpen}
        project={project}
      />
      <HodSyncPackDialog
        onOpenChange={setSyncPackOpen}
        open={syncPackOpen}
        pack={syncPack}
      />
      <ReschedulePreviewDialog
        after={moveProposal?.after}
        before={moveProposal?.before}
        busy={moveBusy}
        onApply={applyTaskMove}
        onOpenChange={(open) => {
          if (!open) setMoveProposal(undefined);
        }}
        open={Boolean(moveProposal)}
        task={moveProposal?.task}
        tasks={tasks}
      />
      {executionTask ? (
        <TaskExecutionPanel
          artifacts={plans.data?.artifacts ?? []}
          details={
            details.find((item) => item.taskId === executionTask.id) ??
            defaultTaskDetails(executionTask)
          }
          executions={plans.data?.executions ?? []}
          onMoveForReview={(task) =>
            mutations.task.mutateAsync(task).then(() => undefined)
          }
          onOpenChange={(open) => {
            if (!open) setExecutionTask(undefined);
          }}
          onSaveArtifact={(artifact) =>
            mutations.artifact.mutateAsync(artifact).then(() => undefined)
          }
          onSaveExecution={(execution) =>
            mutations.execution.mutateAsync(execution).then(() => undefined)
          }
          open={Boolean(executionTask)}
          project={project}
          task={executionTask}
          tasks={tasks}
        />
      ) : null}
    </main>
  );
}
