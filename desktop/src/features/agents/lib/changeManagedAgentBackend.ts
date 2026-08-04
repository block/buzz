import {
  deployManagedAgentToExecutionNode,
  executionReceiptFailure,
} from "@/shared/api/tauriExecution";
import type {
  DeployExecutionWorkloadResponse,
  DeployManagedAgentToExecutionNodeInput,
} from "@/shared/api/tauriExecution";
import { changeManagedAgentBackend } from "@/shared/api/tauriManagedAgents";
import type { ChangeManagedAgentBackendInput } from "@/shared/api/tauriManagedAgents";
import type { ManagedAgent, ManagedAgentBackend } from "@/shared/api/types";
import type { BackendChangeIntent } from "../ui/whereToRunIntent";

type ChangeBackend = (
  input: ChangeManagedAgentBackendInput,
) => Promise<ManagedAgent>;
type DeployExecutionNodeAgent = (
  input: DeployManagedAgentToExecutionNodeInput,
) => Promise<DeployExecutionWorkloadResponse>;

export type BackendChangeResult =
  | { cancelled: true }
  | { cancelled?: false; agent: ManagedAgent };

/** Wire shape (`ManagedAgentBackend`) for an edit-dialog change intent. */
export function backendForChangeIntent(
  intent: BackendChangeIntent,
): ManagedAgentBackend {
  if (intent.type === "local") {
    return { type: "local" };
  }
  if (intent.type === "execution-node") {
    return { type: "execution_node", nodeId: intent.nodeId };
  }
  return { type: "provider", id: intent.id, config: intent.config };
}

/**
 * Swap an existing agent's backend: the Rust transition command performs the
 * orderly teardown of the old body and persists the new binding, then — for
 * execution-node targets — the same authoritative deploy the create flow uses
 * confirms the workload and its identity is projected back onto the summary.
 * Mirrors `createAndDeployExecutionNodeAgent` so receipt semantics cannot
 * drift between create and edit.
 *
 * A deployed legacy provider body has no remote undeploy (protocol v2), so
 * swapping away orphans it — the injectable confirm mirrors delete's orphan
 * warning, and a decline cancels the swap without side effects.
 */
export async function applyManagedAgentBackendChange({
  agent,
  intent,
  runtimeId,
  changeBackend = changeManagedAgentBackend,
  deployExecutionNodeAgent = deployManagedAgentToExecutionNode,
  confirmProviderOrphan = (message: string) => window.confirm(message),
}: {
  agent: ManagedAgent;
  intent: BackendChangeIntent;
  /** Resolved catalog runtime id, required by execution-node deploys. */
  runtimeId?: string;
  changeBackend?: ChangeBackend;
  deployExecutionNodeAgent?: DeployExecutionNodeAgent;
  confirmProviderOrphan?: (message: string) => boolean;
}): Promise<BackendChangeResult> {
  let force = false;
  if (agent.backend.type === "provider" && agent.backendAgentId) {
    const confirmed = confirmProviderOrphan(
      "This agent has a remote provider deployment that cannot be torn down " +
        "automatically. Switching abandons it (it may keep running). Continue?",
    );
    if (!confirmed) {
      return { cancelled: true };
    }
    force = true;
  }

  const updated = await changeBackend({
    pubkey: agent.pubkey,
    backend: backendForChangeIntent(intent),
    runtime: intent.type === "execution-node" ? runtimeId : undefined,
    force,
  });

  if (intent.type !== "execution-node") {
    return { agent: updated };
  }

  const deployment = await deployExecutionNodeAgent({
    pubkey: agent.pubkey,
    nodeId: intent.nodeId,
  });
  const failure = executionReceiptFailure(deployment.receipt);
  if (failure) {
    throw new Error(
      `The execution node rejected the workload command: ${failure}.`,
    );
  }

  return {
    agent: {
      ...updated,
      backendAgentId: deployment.workloadId,
      status: "deployed",
    },
  };
}
