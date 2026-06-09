import {
  createManagedAgent,
  listManagedAgents,
  openDm,
  startManagedAgent,
} from "@/shared/api/tauri";
import {
  meshAgentPreset,
  meshAvailability,
  meshPrepareRelayMeshClient,
  type MeshServeTarget,
} from "@/shared/api/tauriMesh";
import { meshAgentPresetPatch } from "@/features/mesh-compute/applyMeshAgentPreset";
import type { Channel, ManagedAgent } from "@/shared/api/types";

import {
  clearConciergeSelection,
  readConciergeSelection,
  writeConciergeSelection,
} from "./conciergeSelection";
import { CONCIERGE_AGENT_NAME, CONCIERGE_SYSTEM_PROMPT } from "./prompt";

export type ConciergeSession = {
  agent: ManagedAgent;
  /** The persistent DM channel — the Concierge's memory spine. */
  dm: Channel;
  createdAgent: boolean;
  /** True when a saved selection pointed at a deleted agent and we fell
   *  back to provisioning the default — callers should surface this. */
  staleSelection: boolean;
};

/**
 * Resolve the user's chosen Concierge agent by pubkey. Pure — exported for
 * tests. Any agent qualifies (any name, any backend); the Home placement
 * doesn't care what brain it runs. Returns undefined when there is no
 * selection or the selected agent no longer exists (stale selection).
 */
export function resolveConciergeAgent(
  agents: ManagedAgent[],
  selectedPubkey: string | null,
): ManagedAgent | undefined {
  if (!selectedPubkey) return undefined;
  return agents.find((agent) => agent.pubkey === selectedPubkey);
}

/** Pick the mesh serve target to run the Concierge brain on. Pure. */
export function pickMeshTarget(
  targets: MeshServeTarget[],
): MeshServeTarget | undefined {
  return targets[0];
}

/**
 * Selection-first session bootstrap. The user's chosen agent (any name, any
 * backend) gets the Home placement; without a selection we provision the
 * default Concierge on the relay-mesh brain once and record it as the
 * selection — never re-created, never matched by name again. A stale
 * selection (agent deleted) is cleared and falls back to the default path.
 */
export async function ensureConciergeSession(
  selfPubkey: string,
): Promise<ConciergeSession> {
  const agents = await listManagedAgents();
  const selection = readConciergeSelection(selfPubkey);
  let agent = resolveConciergeAgent(agents, selection?.agentPubkey ?? null);
  let createdAgent = false;
  const staleSelection = selection != null && agent === undefined;

  if (staleSelection) clearConciergeSelection(selfPubkey);

  if (!agent) {
    const availability = await meshAvailability();
    const target = pickMeshTarget(availability.serveTargets);
    if (!availability.available || !target) {
      throw new Error(
        availability.reason ??
          "No relay-mesh model is being served. Start serving a model in Settings → Relay compute, then try again.",
      );
    }
    await meshPrepareRelayMeshClient(target.modelId, target);
    const preset = meshAgentPresetPatch(await meshAgentPreset(target.modelId));
    const created = await createManagedAgent({
      name: CONCIERGE_AGENT_NAME,
      ...preset,
      systemPrompt: CONCIERGE_SYSTEM_PROMPT,
      spawnAfterCreate: true,
      // Mesh serve targets aren't persisted; start_managed_agent re-resolves
      // a live target on demand instead of auto-starting with the app.
      startOnAppLaunch: false,
      backend: { type: "local" },
      // Concierge answers its owner only — the DM is a 1:1 surface.
      respondTo: "owner-only",
    });
    agent = created.agent;
    createdAgent = true;
  } else if (agent.status !== "running" && agent.status !== "deployed") {
    // Rust-side preflight re-resolves a live mesh serve target before spawn.
    agent = await startManagedAgent(agent.pubkey);
  }

  // Record (or repair) the selection so future opens bind by pubkey.
  if (!selection || selection.agentPubkey !== agent.pubkey) {
    writeConciergeSelection(selfPubkey, agent.pubkey);
  }

  const dm = await openDm({ pubkeys: [agent.pubkey] });
  return { agent, dm, createdAgent, staleSelection };
}
