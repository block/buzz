import { sendChannelMessage } from "@/shared/api/tauri";
import type {
  Channel,
  ManagedAgent,
  PresenceLookup,
  PresenceStatus,
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

/**
 * The **control-plane** axis: does infrastructure for this agent exist?
 *
 * For remote (provider) agents this is derived from `backend_agent_id` and is deliberately
 * write-once — it stays `deployed` after `!shutdown`, because the provider may have allocated a
 * VM or container that outlives the process. It is bookkeeping, **not liveness**.
 *
 * Use this only for genuinely control-plane questions ("would deleting orphan a deployment?").
 * For "is this thing alive right now", use {@link isManagedAgentLive}.
 */
export function isManagedAgentActive(agent: Pick<ManagedAgent, "status">) {
  return agent.status === "running" || agent.status === "deployed";
}

/** Relay presence for an agent, plus whether presence has loaded at all yet. */
export type ManagedAgentPresence = {
  status: PresenceStatus | undefined;
  loaded: boolean;
};

/**
 * The **live** axis: is this agent's harness connected right now?
 *
 * Per invariant I3 ("Presence is the status", `docs/remote-agents.md`), a remote agent's live
 * state is derived exclusively from relay presence self-signed by the agent key — never from the
 * deployment axis, which never clears without a `undeploy` operation that v1 does not have.
 *
 * Local agents are unaffected: their status comes from a real pid probe, so the control-plane
 * axis *is* liveness for them.
 *
 * While presence is still loading we fall back to the control-plane axis rather than reporting a
 * live agent as dead — otherwise every remote agent would flash "Deploy" on app start. This keeps
 * I3's promise of a *bounded* wrong signal rather than trading one unbounded lie for another.
 */
export function isManagedAgentLive(
  agent: Pick<ManagedAgent, "status" | "backend">,
  presence: ManagedAgentPresence,
): boolean {
  if (agent.backend.type !== "provider") {
    return isManagedAgentActive(agent);
  }

  // Nothing was ever deployed — no presence can make this live.
  if (!isManagedAgentActive(agent)) {
    return false;
  }

  if (!presence.loaded) {
    return true;
  }

  return presence.status === "online" || presence.status === "away";
}

/** Resolve an agent's presence out of a lookup keyed by normalized pubkey. */
export function managedAgentPresence(
  agent: Pick<ManagedAgent, "pubkey">,
  presenceLookup: PresenceLookup | null | undefined,
): ManagedAgentPresence {
  if (!presenceLookup) {
    return { status: undefined, loaded: false };
  }
  return {
    status: presenceLookup[normalizePubkey(agent.pubkey)],
    loaded: true,
  };
}

export function getManagedAgentPrimaryActionLabel(
  agent: ManagedAgent,
  presence: ManagedAgentPresence,
) {
  if (agent.backend.type === "provider") {
    // Deploy converges to at-most-one-live-instance (§Deploy State Machine), so offering
    // "Deploy" for an agent whose deployment record still exists is safe: the provider
    // re-adopts a live instance or recreates a reaped one.
    return isManagedAgentLive(agent, presence) ? "Shutdown" : "Deploy";
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
  onStopped,
}: {
  agent: ManagedAgent;
  startManagedAgent: StartManagedAgent;
  stopManagedAgent: StopManagedAgent;
  /** Called after a successful stop and before start begins — use this to
   * clear stale working badges at the right boundary. */
  onStopped?: () => void;
}) {
  if (agent.backend.type === "local" && isManagedAgentActive(agent)) {
    await stopManagedAgent(agent.pubkey);
    onStopped?.();
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
