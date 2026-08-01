import { invokeTauri } from "@/shared/api/tauri";

/** Safe execution-node projection returned by the Desktop backend. */
export type ExecutionNodeTarget = {
  nodeId: string;
  displayName: string;
  lifecycle: string;
  capabilities: string[];
  observedAt: string;
  availability: "connected" | "unavailable" | "degraded";
};

/** Fetch the currently announced execution nodes. */
export async function listExecutionNodes(): Promise<ExecutionNodeTarget[]> {
  return invokeTauri<ExecutionNodeTarget[]>("list_execution_nodes");
}

/** Safe workload fields accepted by the remote deploy command. */
export type DeployExecutionWorkloadInput = {
  nodeId: string;
  displayName: string;
  runtime: string;
  model?: string;
  provider?: string;
  credentialRefs?: Array<{ provider: string; name: string }>;
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

/** Return the safe error code from a failed or rejected receipt, if present. */
export function executionReceiptFailure(
  receipt: ExecutionReceipt | null,
): string | null {
  return receipt && "error" in receipt.outcome ? receipt.outcome.error : null;
}

/** Publish one encrypted deploy command to a paired execution node. */
export async function deployExecutionWorkload(
  input: DeployExecutionWorkloadInput,
): Promise<DeployExecutionWorkloadResponse> {
  return invokeTauri<DeployExecutionWorkloadResponse>(
    "deploy_execution_workload",
    {
      input: {
        nodeId: input.nodeId,
        displayName: input.displayName,
        runtime: input.runtime,
        model: input.model,
        provider: input.provider,
        credentialRefs: input.credentialRefs ?? [],
      },
    },
  );
}
