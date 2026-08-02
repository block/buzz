import { invokeTauri } from "@/shared/api/tauri";

/** Safe execution-node projection returned by the Desktop backend. */
export type ExecutionNodeTarget = {
  nodeId: string;
  displayName: string;
  lifecycle: string;
  capabilities: string[];
  observedAt: string;
  availability: "connected" | "unavailable" | "degraded";
  workloads: ExecutionWorkloadStatus[];
};

/** Durable workload projection reconstructed from a node announcement. */
export type ExecutionWorkloadStatus = {
  workloadId: string;
  lifecycle: string;
  sequence: number;
};

/** Fetch the currently announced execution nodes. */
export async function listExecutionNodes(): Promise<ExecutionNodeTarget[]> {
  return invokeTauri<ExecutionNodeTarget[]>("list_execution_nodes");
}

/** Input for deploying a persisted managed-agent identity remotely. */
export type DeployManagedAgentToExecutionNodeInput = {
  pubkey: string;
  nodeId: string;
  runtime: string;
  channelId?: string;
};

/** Terminal receipt projection returned after a remote deploy. */
export type ExecutionReceipt = {
  protocolVersion: number;
  nodeId: string;
  requestId: string;
  commandId: string;
  workloadId: string;
  sequence: number;
  outcome:
    | { outcome: "accepted" }
    | { outcome: "progress" }
    | { outcome: "succeeded" }
    | { outcome: "failed"; error: string }
    | { outcome: "rejected"; error: string };
  detail?:
    | {
        detail: "provider_auth_challenge";
        provider: string;
        sessionId: string;
        instructions: string;
      }
    | { detail: "provider_authenticated"; provider: string };
  observedAt: string;
};

/** Result of publishing a deploy command and observing its receipt. */
export type DeployExecutionWorkloadResponse = {
  commandId: string;
  requestId: string;
  workloadId: string;
  nodeId: string;
  publication: { eventId: string; accepted: boolean; message: string };
  receipt: ExecutionReceipt | null;
};

/** Input for a lifecycle command targeting an existing remote workload. */
export type ExecutionWorkloadCommandInput = {
  nodeId: string;
  workloadId: string;
};

/** Return the safe error code from a failed or rejected receipt, if present. */
export function executionReceiptFailure(
  receipt: ExecutionReceipt | null,
): string | null {
  if (!receipt) {
    return "execution node did not confirm the command";
  }
  return "error" in receipt.outcome ? receipt.outcome.error : null;
}

/** Deploy a managed agent while preserving its Desktop identity and config. */
export function deployManagedAgentToExecutionNode(
  input: DeployManagedAgentToExecutionNodeInput,
): Promise<DeployExecutionWorkloadResponse> {
  return invokeTauri<DeployExecutionWorkloadResponse>(
    "deploy_managed_agent_to_execution_node",
    {
      input: {
        pubkey: input.pubkey,
        nodeId: input.nodeId,
        runtime: input.runtime,
        channelId: input.channelId,
      },
    },
  );
}

async function sendExecutionWorkloadCommand(
  command: "start" | "stop" | "restart" | "remove",
  input: ExecutionWorkloadCommandInput,
): Promise<DeployExecutionWorkloadResponse> {
  const response = await invokeTauri<DeployExecutionWorkloadResponse>(
    `${command}_execution_workload`,
    { input },
  );
  const failure = executionReceiptFailure(response.receipt);
  if (failure) {
    throw new Error(failure);
  }
  return response;
}

/** Start a remote workload through its paired execution node. */
export function startExecutionWorkload(
  input: ExecutionWorkloadCommandInput,
): Promise<DeployExecutionWorkloadResponse> {
  return sendExecutionWorkloadCommand("start", input);
}

/** Stop a remote workload through its paired execution node. */
export function stopExecutionWorkload(
  input: ExecutionWorkloadCommandInput,
): Promise<DeployExecutionWorkloadResponse> {
  return sendExecutionWorkloadCommand("stop", input);
}

/** Restart a remote workload through its paired execution node. */
export function restartExecutionWorkload(
  input: ExecutionWorkloadCommandInput,
): Promise<DeployExecutionWorkloadResponse> {
  return sendExecutionWorkloadCommand("restart", input);
}

/** Remove a remote workload through its paired execution node. */
export function removeExecutionWorkload(
  input: ExecutionWorkloadCommandInput,
): Promise<DeployExecutionWorkloadResponse> {
  return sendExecutionWorkloadCommand("remove", input);
}

/** Request an actionable provider-authentication challenge from a node. */
export function startExecutionAuthentication(input: {
  nodeId: string;
  workloadId: string;
  provider: string;
}): Promise<DeployExecutionWorkloadResponse> {
  return invokeTauri<DeployExecutionWorkloadResponse>(
    "start_execution_authentication",
    { input },
  );
}

/** Submit provider-authentication material through the encrypted node command. */
export function submitExecutionAuthentication(input: {
  nodeId: string;
  workloadId: string;
  sessionId: string;
  response: string;
}): Promise<DeployExecutionWorkloadResponse> {
  return invokeTauri<DeployExecutionWorkloadResponse>(
    "submit_execution_authentication",
    { input },
  );
}

/** Cancel a pending provider-authentication session. */
export function cancelExecutionAuthentication(input: {
  nodeId: string;
  workloadId: string;
  sessionId: string;
}): Promise<DeployExecutionWorkloadResponse> {
  return invokeTauri<DeployExecutionWorkloadResponse>(
    "cancel_execution_authentication",
    { input },
  );
}
