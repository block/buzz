import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_MISSION_CONSTRAINT,
  KIND_PLANNING_PLAYBOOK,
  KIND_PLANNING_PROJECT,
  KIND_PLANNING_TASK,
  KIND_PLANNING_TASK_ARTIFACT,
  KIND_PLANNING_TASK_DETAILS,
  KIND_PLANNING_TASK_EXECUTION,
} from "@/shared/constants/kinds";
import {
  buildMissionConstraintEvent,
  buildPlanningPlaybookEvent,
  buildPlanningProjectEvent,
  buildPlanningTaskArtifactEvent,
  buildPlanningTaskDetailsEvent,
  buildPlanningTaskEvent,
  buildPlanningTaskExecutionEvent,
  parseRelayMissionConstraint,
  parseRelayPlanningPlaybook,
  parseRelayPlanningProject,
  parseRelayPlanningTaskArtifact,
  parseRelayPlanningTaskDetails,
  parseRelayPlanningTask,
  parseRelayPlanningTaskExecution,
} from "../domain/eventCodec";
import {
  parseMissionConstraint,
  parsePlanningProject,
  parsePlanningTask,
  type MissionConstraint,
  type PlanningProject,
  type PlanningTask,
} from "../domain/contracts";
import {
  defaultTaskDetails,
  parsePlanningPlaybook,
  parsePlanningTaskArtifact,
  parsePlanningTaskDetails,
  parsePlanningTaskExecution,
  type PlanningPlaybookV1,
  type PlanningTaskArtifactV1,
  type PlanningTaskDetailsV1,
  type PlanningTaskExecutionV1,
} from "../domain/extendedContracts";

type Relay = Pick<typeof relayClient, "fetchEvents" | "publishEvent">;
function newest(events: readonly RelayEvent[]) {
  const result = new Map<string, RelayEvent>();
  for (const event of events) {
    const id = event.tags.find((tag) => tag[0] === "d")?.[1];
    if (!id) continue;
    const prior = result.get(id);
    if (!prior || event.created_at > prior.created_at) result.set(id, event);
  }
  return [...result.values()];
}
async function publish(relay: Relay, event: RelayEvent) {
  return relay.publishEvent(
    event,
    "Timed out persisting planning data.",
    "Failed to persist planning data.",
  );
}
async function priorHead(
  ownerPubkey: string,
  kind: number,
  id: string,
  relay: Relay,
) {
  const events = await relay.fetchEvents({
    kinds: [kind],
    authors: [ownerPubkey],
    limit: 2000,
  });
  return newest(events).find((event) =>
    event.tags.some((tag) => tag[0] === "d" && tag[1] === id),
  );
}
export async function fetchPlans(
  ownerPubkey: string,
  relay: Pick<Relay, "fetchEvents"> = relayClient,
) {
  const [
    projectEvents,
    taskEvents,
    constraintEvents,
    detailsEvents,
    playbookEvents,
    executionEvents,
    artifactEvents,
  ] = await Promise.all([
    relay.fetchEvents({
      kinds: [KIND_PLANNING_PROJECT],
      authors: [ownerPubkey],
      limit: 500,
    }),
    relay.fetchEvents({
      kinds: [KIND_PLANNING_TASK],
      authors: [ownerPubkey],
      limit: 5000,
    }),
    relay.fetchEvents({
      kinds: [KIND_MISSION_CONSTRAINT],
      authors: [ownerPubkey],
      limit: 2000,
    }),
    relay.fetchEvents({
      kinds: [KIND_PLANNING_TASK_DETAILS],
      authors: [ownerPubkey],
      limit: 5000,
    }),
    relay.fetchEvents({
      kinds: [KIND_PLANNING_PLAYBOOK],
      authors: [ownerPubkey],
      limit: 1000,
    }),
    relay.fetchEvents({
      kinds: [KIND_PLANNING_TASK_EXECUTION],
      authors: [ownerPubkey],
      limit: 5000,
    }),
    relay.fetchEvents({
      kinds: [KIND_PLANNING_TASK_ARTIFACT],
      authors: [ownerPubkey],
      limit: 5000,
    }),
  ]);
  const projects = newest(projectEvents)
    .map(parseRelayPlanningProject)
    .filter((item): item is PlanningProject => item !== null);
  const live = new Set(projects.map((project) => project.id));
  const tasks = newest(taskEvents)
    .map(parseRelayPlanningTask)
    .filter(
      (item): item is PlanningTask => item !== null && live.has(item.projectId),
    );
  const constraints = newest(constraintEvents)
    .map(parseRelayMissionConstraint)
    .filter(
      (item): item is MissionConstraint =>
        item !== null && live.has(item.projectId),
    );
  const liveTasks = new Set(tasks.map((task) => task.id));
  const parsedDetails = newest(detailsEvents)
    .map(parseRelayPlanningTaskDetails)
    .filter(
      (item): item is PlanningTaskDetailsV1 =>
        item !== null && live.has(item.projectId) && liveTasks.has(item.taskId),
    );
  const detailByTask = new Map(
    parsedDetails.map((details) => [details.taskId, details]),
  );
  const details = tasks.map(
    (task) => detailByTask.get(task.id) ?? defaultTaskDetails(task),
  );
  const playbooks = newest(playbookEvents)
    .map(parseRelayPlanningPlaybook)
    .filter((item): item is PlanningPlaybookV1 => item !== null);
  const executions = newest(executionEvents)
    .map(parseRelayPlanningTaskExecution)
    .filter(
      (item): item is PlanningTaskExecutionV1 =>
        item !== null && live.has(item.projectId) && liveTasks.has(item.taskId),
    );
  const liveExecutions = new Set(executions.map((execution) => execution.id));
  const artifacts = newest(artifactEvents)
    .map(parseRelayPlanningTaskArtifact)
    .filter(
      (item): item is PlanningTaskArtifactV1 =>
        item !== null &&
        live.has(item.projectId) &&
        liveTasks.has(item.taskId) &&
        liveExecutions.has(item.executionId),
    );
  return Object.freeze({
    projects: Object.freeze(projects),
    tasks: Object.freeze(tasks),
    constraints: Object.freeze(constraints),
    details: Object.freeze(details),
    playbooks: Object.freeze(playbooks),
    executions: Object.freeze(executions),
    artifacts: Object.freeze(artifacts),
  });
}
export async function publishPlanningProject(
  ownerPubkey: string,
  input: PlanningProject,
  relay: Relay = relayClient,
) {
  const project = parsePlanningProject(input);
  const prior = await priorHead(
    ownerPubkey,
    KIND_PLANNING_PROJECT,
    project.id,
    relay,
  );
  return publish(
    relay,
    await buildPlanningProjectEvent(project, prior?.created_at),
  );
}
export async function publishPlanningTask(
  ownerPubkey: string,
  input: PlanningTask,
  relay: Relay = relayClient,
) {
  const task = parsePlanningTask(input);
  const prior = await priorHead(
    ownerPubkey,
    KIND_PLANNING_TASK,
    task.id,
    relay,
  );
  return publish(relay, await buildPlanningTaskEvent(task, prior?.created_at));
}
export async function publishMissionConstraint(
  ownerPubkey: string,
  input: MissionConstraint,
  relay: Relay = relayClient,
) {
  const constraint = parseMissionConstraint(input);
  const prior = await priorHead(
    ownerPubkey,
    KIND_MISSION_CONSTRAINT,
    constraint.id,
    relay,
  );
  return publish(
    relay,
    await buildMissionConstraintEvent(constraint, prior?.created_at),
  );
}
export async function publishPlanningTaskDetails(
  ownerPubkey: string,
  input: PlanningTaskDetailsV1,
  relay: Relay = relayClient,
) {
  const details = parsePlanningTaskDetails(input);
  const prior = await priorHead(
    ownerPubkey,
    KIND_PLANNING_TASK_DETAILS,
    details.id,
    relay,
  );
  return publish(
    relay,
    await buildPlanningTaskDetailsEvent(details, prior?.created_at),
  );
}
export async function publishPlanningPlaybook(
  ownerPubkey: string,
  input: PlanningPlaybookV1,
  relay: Relay = relayClient,
) {
  const playbook = parsePlanningPlaybook(input);
  const prior = await priorHead(
    ownerPubkey,
    KIND_PLANNING_PLAYBOOK,
    playbook.id,
    relay,
  );
  return publish(
    relay,
    await buildPlanningPlaybookEvent(playbook, prior?.created_at),
  );
}
export async function publishPlanningTaskExecution(
  ownerPubkey: string,
  input: PlanningTaskExecutionV1,
  relay: Relay = relayClient,
) {
  const execution = parsePlanningTaskExecution(input);
  const prior = await priorHead(
    ownerPubkey,
    KIND_PLANNING_TASK_EXECUTION,
    execution.id,
    relay,
  );
  return publish(
    relay,
    await buildPlanningTaskExecutionEvent(execution, prior?.created_at),
  );
}
export async function publishPlanningTaskArtifact(
  ownerPubkey: string,
  input: PlanningTaskArtifactV1,
  relay: Relay = relayClient,
) {
  const artifact = parsePlanningTaskArtifact(input);
  const prior = await priorHead(
    ownerPubkey,
    KIND_PLANNING_TASK_ARTIFACT,
    artifact.id,
    relay,
  );
  return publish(
    relay,
    await buildPlanningTaskArtifactEvent(artifact, prior?.created_at),
  );
}
export function assertProjectCanDelete(
  projectId: string,
  tasks: readonly PlanningTask[],
  constraints: readonly MissionConstraint[],
) {
  if (
    tasks.some((task) => task.projectId === projectId) ||
    constraints.some((constraint) => constraint.projectId === projectId)
  )
    throw new Error(
      "Remove or transfer this project's tasks and constraints before deleting it.",
    );
}
