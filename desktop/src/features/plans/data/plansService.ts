import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_MISSION_CONSTRAINT,
  KIND_PLANNING_PROJECT,
  KIND_PLANNING_TASK,
} from "@/shared/constants/kinds";
import {
  buildMissionConstraintEvent,
  buildPlanningProjectEvent,
  buildPlanningTaskEvent,
  parseRelayMissionConstraint,
  parseRelayPlanningProject,
  parseRelayPlanningTask,
} from "../domain/eventCodec";
import {
  parseMissionConstraint,
  parsePlanningProject,
  parsePlanningTask,
  type MissionConstraint,
  type PlanningProject,
  type PlanningTask,
} from "../domain/contracts";

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
  const [projectEvents, taskEvents, constraintEvents] = await Promise.all([
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
  return Object.freeze({
    projects: Object.freeze(projects),
    tasks: Object.freeze(tasks),
    constraints: Object.freeze(constraints),
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
