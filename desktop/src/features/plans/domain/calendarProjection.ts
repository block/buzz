import type { PlanningProject, PlanningTask } from "./contracts";

export type PlanTaskCalendarProjection = Readonly<{
  kind: "planTask";
  id: string;
  title: string;
  date: string;
  allDay: true;
  visualStatus: PlanningTask["status"];
  owner: string;
  projectId: string;
  taskId: string;
  href: string;
}>;

export function projectTaskMilestone(
  task: PlanningTask,
  project: PlanningProject,
): PlanTaskCalendarProjection | null {
  if (
    project.status !== "active" ||
    task.projectId !== project.id ||
    task.isSummary ||
    task.status === "cancelled" ||
    task.dueDate === null
  ) {
    return null;
  }
  return Object.freeze({
    kind: "planTask",
    id: `plan-task:${task.id}`,
    title: `${task.wbs} ${task.title}`,
    date: task.dueDate,
    allDay: true,
    visualStatus: task.status,
    owner: task.owner,
    projectId: project.id,
    taskId: task.id,
    href: `/plans/${encodeURIComponent(project.id)}?task=${encodeURIComponent(task.id)}`,
  });
}

export function projectTaskMilestones(
  tasks: readonly PlanningTask[],
  projects: readonly PlanningProject[],
): readonly PlanTaskCalendarProjection[] {
  const byId = new Map(projects.map((project) => [project.id, project]));
  return tasks
    .map((task) => {
      const project = byId.get(task.projectId);
      return project ? projectTaskMilestone(task, project) : null;
    })
    .filter(
      (milestone): milestone is PlanTaskCalendarProjection =>
        milestone !== null,
    )
    .sort(
      (left, right) =>
        left.date.localeCompare(right.date) ||
        left.title.localeCompare(right.title),
    );
}
