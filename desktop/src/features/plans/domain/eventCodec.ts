import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_MISSION_CONSTRAINT,
  KIND_PLANNING_PROJECT,
  KIND_PLANNING_TASK,
} from "@/shared/constants/kinds";
import {
  parseMissionConstraint,
  parsePlanningProject,
  parsePlanningTask,
  type MissionConstraint,
  type PlanningProject,
  type PlanningTask,
} from "./contracts";

const nowAfter = (prior?: number) =>
  Math.max(Math.floor(Date.now() / 1000), (prior ?? 0) + 1);
let signer = signRelayEvent;
export function setPlanningEventSignerForTests(
  replacement: typeof signRelayEvent | undefined,
) {
  signer = replacement ?? signRelayEvent;
}
const content = (value: unknown) => JSON.stringify(value);
const tag = (event: RelayEvent, name: string) =>
  event.tags.find((item) => item[0] === name)?.[1];

export async function buildPlanningProjectEvent(
  input: PlanningProject,
  priorCreatedAt?: number,
) {
  const project = parsePlanningProject(input);
  return signer({
    kind: KIND_PLANNING_PROJECT,
    content: content(project),
    createdAt: nowAfter(priorCreatedAt),
    tags: [
      ["d", project.id],
      ["ready", project.missionReadyDate],
    ],
  });
}
export async function buildPlanningTaskEvent(
  input: PlanningTask,
  priorCreatedAt?: number,
) {
  const task = parsePlanningTask(input);
  const tags = [
    ["d", task.id],
    ["project", task.projectId],
  ];
  if (task.dueDate) tags.push(["due", task.dueDate]);
  return signer({
    kind: KIND_PLANNING_TASK,
    content: content(task),
    createdAt: nowAfter(priorCreatedAt),
    tags,
  });
}
export async function buildMissionConstraintEvent(
  input: MissionConstraint,
  priorCreatedAt?: number,
) {
  const constraint = parseMissionConstraint(input);
  const tags = [
    ["d", constraint.id],
    ["project", constraint.projectId],
  ];
  if (constraint.requiredDate) tags.push(["due", constraint.requiredDate]);
  return signer({
    kind: KIND_MISSION_CONSTRAINT,
    content: content(constraint),
    createdAt: nowAfter(priorCreatedAt),
    tags,
  });
}
export function parseRelayPlanningProject(event: RelayEvent) {
  if (event.kind !== KIND_PLANNING_PROJECT || !tag(event, "d")) return null;
  try {
    const value = parsePlanningProject(JSON.parse(event.content));
    return value.id === tag(event, "d") ? value : null;
  } catch {
    return null;
  }
}
export function parseRelayPlanningTask(event: RelayEvent) {
  if (
    event.kind !== KIND_PLANNING_TASK ||
    !tag(event, "d") ||
    !tag(event, "project")
  )
    return null;
  try {
    const value = parsePlanningTask(JSON.parse(event.content));
    return value.id === tag(event, "d") &&
      value.projectId === tag(event, "project") &&
      (value.dueDate ?? undefined) === tag(event, "due")
      ? value
      : null;
  } catch {
    return null;
  }
}
export function parseRelayMissionConstraint(event: RelayEvent) {
  if (
    event.kind !== KIND_MISSION_CONSTRAINT ||
    !tag(event, "d") ||
    !tag(event, "project")
  )
    return null;
  try {
    const value = parseMissionConstraint(JSON.parse(event.content));
    return value.id === tag(event, "d") &&
      value.projectId === tag(event, "project") &&
      (value.requiredDate ?? undefined) === tag(event, "due")
      ? value
      : null;
  } catch {
    return null;
  }
}
