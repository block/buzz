import type { AgentWorkingState } from "@/features/agents/agentWorkingSignal";
import type { ObserverEvent } from "@/features/agents/ui/agentSessionTypes";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  type AgentLifecycle,
  type AgentLocationReason,
  type CollaborationContext,
  LIVING_SHIP_ADVISERS,
  type LivingShipAgentId,
  type ShipLocationId,
  type ShipRoomId,
  SHIP_ROOMS,
  resolveAgentLocation,
} from "./shipLayout";

type ManagedAgentProjectionInput = Pick<
  ManagedAgent,
  "pubkey" | "name" | "personaId" | "status"
>;

export type LivingShipAgentPresentation = {
  adviser: LivingShipAgentId;
  personaId: string;
  pubkey: string;
  name: string;
  label: string;
  shortLabel: string;
  spriteColumn: number;
  lifecycle: AgentLifecycle;
  working: boolean;
  locationId: ShipLocationId;
  locationReason: AgentLocationReason;
  channelId: string | null;
  channelName: string | null;
  workingSince: number | null;
  taskSummary: string | null;
  collaborationId: string | null;
  collaboratorPubkeys: string[];
};

type CollaborationMetadata = {
  id: string;
  workspace: ShipRoomId | null;
  context: CollaborationContext | string | null;
  participantPubkeys: string[];
  summary: string | null;
};

const ROOM_IDS = new Set<ShipRoomId>(SHIP_ROOMS.map((room) => room.id));

function stringField(
  record: Record<string, unknown>,
  ...keys: string[]
): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function stringListField(
  record: Record<string, unknown>,
  ...keys: string[]
): string[] {
  for (const key of keys) {
    const value = record[key];
    if (Array.isArray(value)) {
      return value.filter(
        (entry): entry is string =>
          typeof entry === "string" && entry.trim().length > 0,
      );
    }
  }
  return [];
}

export function collaborationMetadataFromEvent(
  event: ObserverEvent | undefined,
): CollaborationMetadata | null {
  if (!event) return null;
  const envelope = event.collaboration ?? event.payload;
  if (typeof envelope !== "object" || envelope === null) return null;
  const record = envelope as Record<string, unknown>;
  const id = stringField(record, "collaborationId", "collaboration_id", "id");
  if (!id) return null;
  const rawWorkspace = stringField(record, "workspace");
  return {
    id,
    workspace:
      rawWorkspace && ROOM_IDS.has(rawWorkspace as ShipRoomId)
        ? (rawWorkspace as ShipRoomId)
        : null,
    context: stringField(record, "context"),
    participantPubkeys: stringListField(
      record,
      "participantPubkeys",
      "participant_pubkeys",
    ),
    summary: stringField(record, "summary", "taskSummary", "task_summary"),
  };
}

function latestActiveEvent(
  events: readonly ObserverEvent[] | undefined,
  channelId: string | null,
): ObserverEvent | undefined {
  if (!events) return undefined;
  return [...events]
    .reverse()
    .find(
      (event) =>
        event.kind !== "turn_completed" &&
        event.kind !== "turn_error" &&
        event.kind !== "agent_panic" &&
        (channelId === null || event.channelId === channelId),
    );
}

function lifecycleForStatus(
  status: ManagedAgentProjectionInput["status"],
): AgentLifecycle {
  return status === "running" || status === "deployed" ? "online" : "offline";
}

export function projectLivingShipAgents(input: {
  managedAgents: readonly ManagedAgentProjectionInput[];
  channels: readonly { id: string; name: string }[];
  workingByPubkey: ReadonlyMap<
    string,
    Pick<AgentWorkingState, "working" | "channels">
  >;
  observerEventsByPubkey: ReadonlyMap<string, readonly ObserverEvent[]>;
}): LivingShipAgentPresentation[] {
  const agentByPersona = new Map(
    input.managedAgents
      .filter((agent) => agent.personaId)
      .map((agent) => [agent.personaId as string, agent] as const),
  );
  const channelNames = new Map(
    input.channels.map((channel) => [channel.id, channel.name] as const),
  );

  const projected = LIVING_SHIP_ADVISERS.flatMap((visual) => {
    const managedAgent = agentByPersona.get(visual.personaId);
    if (!managedAgent) return [];
    const key = normalizePubkey(managedAgent.pubkey);
    const working = input.workingByPubkey.get(managedAgent.pubkey) ??
      input.workingByPubkey.get(key) ?? { working: false, channels: [] };
    const primaryChannel = working.channels[0] ?? null;
    const events =
      input.observerEventsByPubkey.get(managedAgent.pubkey) ??
      input.observerEventsByPubkey.get(key);
    const collaboration = collaborationMetadataFromEvent(
      latestActiveEvent(events, primaryChannel?.channelId ?? null),
    );
    const lifecycle = lifecycleForStatus(managedAgent.status);
    const location = resolveAgentLocation({
      adviser: visual.adviser,
      lifecycle,
      working: working.working,
      collaboration,
    });

    return [
      {
        adviser: visual.adviser,
        personaId: visual.personaId,
        pubkey: managedAgent.pubkey,
        name: managedAgent.name,
        label: visual.label,
        shortLabel: visual.shortLabel,
        spriteColumn: visual.spriteColumn,
        lifecycle,
        working: working.working,
        locationId: location.locationId,
        locationReason: location.reason,
        channelId: primaryChannel?.channelId ?? null,
        channelName: primaryChannel
          ? (channelNames.get(primaryChannel.channelId) ?? null)
          : null,
        workingSince: primaryChannel?.anchorAt ?? null,
        taskSummary: collaboration?.summary ?? null,
        collaborationId: collaboration?.id ?? null,
        collaboratorPubkeys: [],
      } satisfies LivingShipAgentPresentation,
    ];
  });

  return projected.map((agent) => {
    if (!agent.collaborationId) return agent;
    const explicit = collaborationMetadataFromEvent(
      latestActiveEvent(
        input.observerEventsByPubkey.get(agent.pubkey),
        agent.channelId,
      ),
    )?.participantPubkeys;
    const collaborators = projected
      .filter(
        (candidate) =>
          candidate.pubkey !== agent.pubkey &&
          candidate.collaborationId === agent.collaborationId &&
          (!explicit ||
            explicit.length === 0 ||
            explicit.some(
              (pubkey) =>
                normalizePubkey(pubkey) === normalizePubkey(candidate.pubkey),
            )),
      )
      .map((candidate) => candidate.pubkey);
    return { ...agent, collaboratorPubkeys: collaborators };
  });
}
