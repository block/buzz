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

export function isManagedAgentActive(agent: Pick<ManagedAgent, "status">) {
  return agent.status === "running" || agent.status === "deployed";
}

/**
 * The presence a surface should show for an agent the control plane calls
 * active — `isManagedAgentActive` says what this desktop DID to the agent, not
 * whether it is alive right now, and for a remote deployment those two facts
 * diverge permanently.
 *
 * A local record's `"running"` is this machine's own process table, so the
 * control plane is the liveness answer for it and stays authoritative: the
 * relay may not retain ephemeral kind:20001 presence at all, and a relay blip
 * must not make a process we are supervising read as dead.
 *
 * A provider-backed record's `"deployed"` says only that the deploy
 * succeeded. `backend_agent_id` is written once, on that success, and nothing
 * clears it — the provider protocol has no undeploy — so a remote agent that
 * died hours ago is still `"deployed"` forever. The relay is the only channel
 * that knows, which is exactly what `deleteManagedAgentWithRules` already
 * trusts before it warns about orphaning a deployment. Silence there means
 * "not known to be alive", so it reads offline rather than claiming otherwise.
 */
export function managedAgentPresenceStatus(
  agent: Pick<ManagedAgent, "backend" | "pubkey">,
  presenceLookup: PresenceLookup | null | undefined,
): PresenceStatus {
  if (agent.backend.type === "local") return "online";
  return presenceLookup?.[normalizePubkey(agent.pubkey)] ?? "offline";
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

/**
 * Delete a managed-agent record, sending `!shutdown` first when the agent is a
 * live provider deployment that can still be reached through a channel.
 *
 * `remoteOrphanDisclosedByCaller` asserts that the caller has ALREADY shown the
 * user a confirmation naming this agent's remote unit and stating that the
 * delete does not stop it. It suppresses this function's fallback
 * `window.confirm` only — it never suppresses `!shutdown`, and it is not a
 * "delete quietly" switch.
 *
 * It exists because two surfaces would otherwise stack two dialogs on one
 * click: the profile panel's `AgentDeleteConfirmDialog` already carries the
 * full disclosure. The name is deliberately a claim about the caller rather
 * than a `skip…` verb, because the previous spelling let that caller's copy
 * drift into promising a remote teardown that the provider protocol has never
 * implemented, with nothing tying the two together.
 */
export async function deleteManagedAgentWithRules({
  agent,
  channels,
  deleteManagedAgent,
  preferredChannelId,
  presenceLookup,
  relayAgents,
  remoteOrphanDisclosedByCaller = false,
}: {
  agent: ManagedAgent;
  deleteManagedAgent: DeleteManagedAgent;
  remoteOrphanDisclosedByCaller?: boolean;
} & ManagedAgentActionContext): Promise<ManagedAgentActionResult> {
  if (agent.backend.type === "provider" && agent.backendAgentId) {
    const presence = presenceLookup?.[normalizePubkey(agent.pubkey)];
    const channelId = resolveManagedAgentChannelId(agent, {
      channels,
      preferredChannelId,
      relayAgents,
    });
    const reachable =
      channelId !== null && (presence === "online" || presence === "away");

    // Best-effort graceful stop. Only possible while the agent is both online
    // and addressable in a channel — the shutdown travels as a mention, so
    // there is no out-of-band path to it. Sent regardless of whether the
    // caller already confirmed: it is a courtesy to the agent, not a prompt.
    if (reachable && channelId) {
      await sendChannelMessage(channelId, "!shutdown", undefined, undefined, [
        agent.pubkey,
      ]);
    }

    if (!remoteOrphanDisclosedByCaller) {
      // Every branch says the same thing, because the same thing is true in
      // every branch: this app cannot tear down a remote deployment. Only the
      // reason it cannot promise a clean stop differs.
      const situation = reachable
        ? "A shutdown command was sent, but it may not have completed."
        : channelId
          ? "This agent is not responding, so no shutdown could be delivered."
          : "This agent is not in any channel, so no shutdown could be delivered.";
      const confirmed = window.confirm(
        `${situation} Deleting removes the local record but does not stop the ` +
          `remote deployment — ${agent.backendAgentId} keeps running until ` +
          `stopped on the host. Continue?`,
      );
      if (!confirmed) {
        return { cancelled: true };
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
