import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, FileUp, Plus } from "lucide-react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  calculatePlanSchedule,
  type PlanningSchedule,
} from "@/shared/api/tauriPlans";
import { usePlanMutations, usePlansQuery } from "../hooks";
import type { MissionConstraint, PlanningTask } from "../domain/contracts";
import { ConstraintEditorDialog } from "./ConstraintEditorDialog";
import { GanttChart } from "./GanttChart";
import { MissionConstraintsPanel } from "./MissionConstraintsPanel";
import { TaskEditorDialog } from "./TaskEditorDialog";
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
  const [editingConstraint, setEditingConstraint] =
    React.useState<MissionConstraint>();
  const [validationError, setValidationError] = React.useState<string>();
  React.useEffect(() => {
    if (!selectedTaskId || !tasks.length) return;
    const selected = tasks.find((task) => task.id === selectedTaskId);
    if (selected) {
      setEditingTask(selected);
      setTaskOpen(true);
    }
  }, [selectedTaskId, tasks]);
  async function saveTask(task: PlanningTask) {
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
        <section>
          <div className="mb-2 flex items-center justify-between">
            <h2 className="text-base font-semibold">Work breakdown</h2>
            <span className="text-xs text-muted-foreground">
              Select a row to edit
            </span>
          </div>
          <TaskTable
            onEdit={(task) => {
              setEditingTask(task);
              setTaskOpen(true);
            }}
            schedule={scheduleValue?.tasks ?? []}
            tasks={tasks}
          />
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
    </main>
  );
}
