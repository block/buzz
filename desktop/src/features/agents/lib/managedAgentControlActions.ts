import { sendChannelMessage } from "@/shared/api/tauri";
import type {
  Channel,
  ManagedAgent,
  PresenceLookup,
  RelayAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type DeleteManagedAgentInput = {
  pubkey: string;
  forceRemoteDelete?: boolean;
};

type StartManagedAgent = (pubkey: string) => Promise<unknown>;
type StopManagedAgent = (pubkey: string) => Promise<unknown>;
type DeleteManagedAgent = (input: DeleteManagedAgentInput) => Promise<unknown>;

type ManagedAgentChannelContext = {
  channels: readonly Channel[];
  preferredChannelId?: string | null;
  relayAgents: readonly RelayAgent[];
};

type ManagedAgentActionContext = ManagedAgentChannelContext & {
  presenceLookup?: PresenceLookup | null;
};

export type ManagedAgentActionResult = {
  cancelled?: boolean;
  noticeMessage?: string;
};

export function isManagedAgentActive(agent: Pick<ManagedAgent, "status">) {
  return agent.status === "running" || agent.status === "deployed";
}

export function getManagedAgentPrimaryActionLabel(agent: ManagedAgent) {
  if (agent.backend.type === "provider") {
    return isManagedAgentActive(agent) ? "Shutdown" : "Deploy";
  }

  if (isManagedAgentActive(agent)) {
    return "Stop";
  }

  return agent.status === "stopped" ? "Respawn" : "Spawn";
}

export function resolveManagedAgentChannelId(
  agent: Pick<ManagedAgent, "pubkey">,
  context: ManagedAgentChannelContext,
) {
  if (context.preferredChannelId) {
    return context.preferredChannelId;
  }

  const relayAgent = context.relayAgents.find(
    (candidate) =>
      normalizePubkey(candidate.pubkey) === normalizePubkey(agent.pubkey),
  );

  if (relayAgent?.channelIds?.length) {
    return relayAgent.channelIds[0];
  }

  const channelName = relayAgent?.channels?.[0];
  if (!channelName) {
    return null;
  }

  const matches = context.channels.filter(
    (channel) => channel.name === channelName,
  );
  return matches.length === 1 ? matches[0].id : null;
}

export async function startManagedAgentWithRules({
  agent,
  startManagedAgent,
}: {
  agent: ManagedAgent;
  startManagedAgent: StartManagedAgent;
}) {
  // Relay-mesh agents are no longer blocked here: the backend start preflight
  // (ensure_relay_mesh_for_record) re-resolves a live serve target and dials
  // it, failing with an actionable error when no peer serves the model.
  await startManagedAgent(agent.pubkey);
}

export async function respawnManagedAgentWithRules({
  agent,
  startManagedAgent,
  stopManagedAgent,
}: {
  agent: ManagedAgent;
  startManagedAgent: StartManagedAgent;
  stopManagedAgent: StopManagedAgent;
}) {
  if (agent.backend.type === "local" && isManagedAgentActive(agent)) {
    await stopManagedAgent(agent.pubkey);
  }

  await startManagedAgent(agent.pubkey);
}

export async function stopManagedAgentWithRules({
  agent,
  channels,
  preferredChannelId,
  relayAgents,
  stopManagedAgent,
}: {
  agent: ManagedAgent;
  stopManagedAgent: StopManagedAgent;
} & ManagedAgentChannelContext): Promise<ManagedAgentActionResult> {
  if (agent.backend.type === "provider") {
    const channelId = resolveManagedAgentChannelId(agent, {
      channels,
      preferredChannelId,
      relayAgents,
    });
    if (!channelId) {
      throw new Error("Cannot stop: agent is not in any channel");
    }

    await sendChannelMessage(channelId, "!shutdown", undefined, undefined, [
      agent.pubkey,
    ]);
    return {
      noticeMessage: "Shutdown command sent. Agent will stop shortly.",
    };
  }

  await stopManagedAgent(agent.pubkey);
  return {};
}

export async function deleteManagedAgentWithRules({
  agent,
  channels,
  deleteManagedAgent,
  preferredChannelId,
  presenceLookup,
  relayAgents,
  skipRemoteDeleteConfirm = false,
}: {
  agent: ManagedAgent;
  deleteManagedAgent: DeleteManagedAgent;
  skipRemoteDeleteConfirm?: boolean;
} & ManagedAgentActionContext): Promise<ManagedAgentActionResult> {
  if (agent.backend.type === "provider" && agent.backendAgentId) {
    const presence = presenceLookup?.[normalizePubkey(agent.pubkey)];
    const channelId = resolveManagedAgentChannelId(agent, {
      channels,
      preferredChannelId,
      relayAgents,
    });
    const backendLabel = agent.backend.id;
    const leaseLabel = agent.backendAgentId;

    if (channelId) {
      if (presence === "online" || presence === "away") {
        await sendChannelMessage(channelId, "!shutdown", undefined, undefined, [
          agent.pubkey,
        ]);

        if (!skipRemoteDeleteConfirm) {
          const confirmed = window.confirm(
            `Shutdown sent to the agent. Delete will also ask ${backendLabel} ` +
              `to release remote capacity (${leaseLabel}). Continue?`,
          );
          if (!confirmed) {
            return { cancelled: true };
          }
        }
      } else {
        if (!skipRemoteDeleteConfirm) {
          const confirmed = window.confirm(
            `This agent is offline. Delete will remove the local record and ask ` +
              `${backendLabel} to release ${leaseLabel}. Continue?`,
          );
          if (!confirmed) {
            return { cancelled: true };
          }
        }
      }
    } else {
      if (!skipRemoteDeleteConfirm) {
        const confirmed = window.confirm(
          `This agent is on ${backendLabel} (${leaseLabel}) but not in any channel. ` +
            `Delete will remove the local record and try to release remote capacity. Continue?`,
        );
        if (!confirmed) {
          return { cancelled: true };
        }
      }
    }
  }

  const isDeployedRemote =
    agent.backend.type === "provider" && agent.backendAgentId;
  await deleteManagedAgent({
    pubkey: agent.pubkey,
    forceRemoteDelete: isDeployedRemote ? true : undefined,
  });

  return {};
}
