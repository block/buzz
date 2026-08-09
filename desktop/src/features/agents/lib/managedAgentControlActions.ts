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

  return agent.status === "stopped" ? "Restart Agent" : "Start Agent";
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

/**
 * Entity holon R2 / upstream #2857 spirit:
 * If the same DNA (pubkey) is already online/away on the relay, starting a
 * second local body is dual-spawn. Fail closed unless allowDualBody.
 */
export function refuseDualBodyIfPresentElsewhere(input: {
  agent: Pick<ManagedAgent, "pubkey" | "name" | "backend" | "status">;
  presenceLookup?: PresenceLookup | null;
  /** Optional public place hint (host · role · surface_kind) — never paths. */
  placeHint?: {
    hostId?: string;
    hostRole?: string;
    surfaceKind?: string;
  } | null;
  /** When true, skip the guard (explicit fork path later). */
  allowDualBody?: boolean;
}): void {
  if (input.allowDualBody) return;
  // Provider deploy has its own at-most-one converge; local is the absently-Respawn risk.
  if (input.agent.backend.type !== "local") return;
  if (isManagedAgentActive(input.agent)) return;

  const pk = normalizePubkey(input.agent.pubkey);
  const status =
    input.presenceLookup?.[pk] ?? input.presenceLookup?.[input.agent.pubkey];
  if (status !== "online" && status !== "away") return;

  const short = pk.slice(0, 8);
  const place = input.placeHint;
  const placeBits = [place?.hostRole, place?.hostId, place?.surfaceKind]
    .filter(Boolean)
    .join(" · ");
  throw new Error(
    `Refuse dual body for ${input.agent.name} (DNA ${short}…): already ${status}` +
      (placeBits ? ` · ${placeBits}` : " elsewhere") +
      ". Stop the other body or use a fork with a new birth certificate — " +
      "do not absently Respawn on this computer expecting to continue a remote workspace.",
  );
}

export async function startManagedAgentWithRules({
  agent,
  startManagedAgent,
  presenceLookup,
  placeHint,
  allowDualBody,
}: {
  agent: ManagedAgent;
  startManagedAgent: StartManagedAgent;
  presenceLookup?: PresenceLookup | null;
  placeHint?: {
    hostId?: string;
    hostRole?: string;
    surfaceKind?: string;
  } | null;
  allowDualBody?: boolean;
}) {
  refuseDualBodyIfPresentElsewhere({
    agent,
    presenceLookup,
    placeHint,
    allowDualBody,
  });
  // Relay-mesh agents are no longer blocked here: the backend start preflight
  // (ensure_relay_mesh_for_record) re-resolves a live serve target and dials
  // it, failing with an actionable error when no peer serves the model.
  await startManagedAgent(agent.pubkey);
}

export async function respawnManagedAgentWithRules({
  agent,
  startManagedAgent,
  stopManagedAgent,
  onStopped,
  presenceLookup,
  placeHint,
  allowDualBody,
}: {
  agent: ManagedAgent;
  startManagedAgent: StartManagedAgent;
  stopManagedAgent: StopManagedAgent;
  /** Called after a successful stop and before start begins — use this to
   * clear stale working badges at the right boundary. */
  onStopped?: () => void;
  presenceLookup?: PresenceLookup | null;
  placeHint?: {
    hostId?: string;
    hostRole?: string;
    surfaceKind?: string;
  } | null;
  allowDualBody?: boolean;
}) {
  if (agent.backend.type === "local" && isManagedAgentActive(agent)) {
    await stopManagedAgent(agent.pubkey);
    onStopped?.();
    // Local stop then start is same-host restart — not dual-body.
    await startManagedAgent(agent.pubkey);
    return;
  }

  refuseDualBodyIfPresentElsewhere({
    agent,
    presenceLookup,
    placeHint,
    allowDualBody,
  });
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

    if (channelId) {
      if (presence === "online" || presence === "away") {
        await sendChannelMessage(channelId, "!shutdown", undefined, undefined, [
          agent.pubkey,
        ]);

        if (!skipRemoteDeleteConfirm) {
          const confirmed = window.confirm(
            "Shutdown command sent, but the agent may still be running. " +
              "Deleting now removes the local record — the remote deployment " +
              "will be orphaned if shutdown hasn't completed. Continue?",
          );
          if (!confirmed) {
            return { cancelled: true };
          }
        }
      } else {
        if (!skipRemoteDeleteConfirm) {
          const confirmed = window.confirm(
            "This agent is offline but the remote deployment may still exist. " +
              "Deleting removes the local management record. Continue?",
          );
          if (!confirmed) {
            return { cancelled: true };
          }
        }
      }
    } else {
      if (!skipRemoteDeleteConfirm) {
        const confirmed = window.confirm(
          "This agent is deployed but not in any channel. " +
            "Deleting will orphan the remote deployment (it will keep running). Continue?",
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
