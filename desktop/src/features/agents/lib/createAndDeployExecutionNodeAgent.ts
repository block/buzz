import {
  deployManagedAgentToExecutionNode,
  executionReceiptFailure,
} from "@/shared/api/tauriExecution";
import type {
  DeployExecutionWorkloadResponse,
  DeployManagedAgentToExecutionNodeInput,
} from "@/shared/api/tauriExecution";
import type {
  CreateManagedAgentInput,
  CreateManagedAgentResponse,
} from "@/shared/api/types";

type CreateManagedAgent = (
  input: CreateManagedAgentInput,
) => Promise<CreateManagedAgentResponse>;
type DeployExecutionNodeAgent = (
  input: DeployManagedAgentToExecutionNodeInput,
) => Promise<DeployExecutionWorkloadResponse>;

/**
 * Create the durable identity before handing it to an execution node, then
 * project the confirmed workload identity back onto the created agent.
 *
 * Every caller uses this boundary so receipt semantics and the deployed UI
 * state cannot drift between persona-dialog and agent-management starts.
 */
export async function createAndDeployExecutionNodeAgent({
  input,
  createManagedAgent,
  deployExecutionNodeAgent = deployManagedAgentToExecutionNode,
  nodeId,
  channelId,
}: {
  input: CreateManagedAgentInput;
  createManagedAgent: CreateManagedAgent;
  deployExecutionNodeAgent?: DeployExecutionNodeAgent;
  nodeId: string;
  channelId?: string;
}): Promise<CreateManagedAgentResponse> {
  const created = await createManagedAgent(input);
  if (created.spawnError) {
    throw new Error(created.spawnError);
  }

  const deployment = await deployExecutionNodeAgent({
    pubkey: created.agent.pubkey,
    nodeId,
    channelId,
  });
  const failure = executionReceiptFailure(deployment.receipt);
  if (failure) {
    throw new Error(
      `The execution node rejected the workload command: ${failure}.`,
    );
  }

  return {
    ...created,
    agent: {
      ...created.agent,
      backendAgentId: deployment.workloadId,
      status: "deployed",
    },
  };
}
