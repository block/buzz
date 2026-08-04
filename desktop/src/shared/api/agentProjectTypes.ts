import type { ProjectConnectionScope } from "./projectConnectionTypes";

export type AgentProjectScope = {
  relayUrl: string;
  operatorPubkey: string;
  /**
   * Durable NIP-MP Project coordinate. Legacy one-repository Projects use
   * their NIP-34 repository coordinate.
   */
  projectAddress: string;
  /** Project discussion channel used to scope agent traffic. */
  channelId: string;
};

export type AgentToolRequirement = {
  /** Stable template-local key used by deployed-agent bindings. */
  id: string;
  /** Plain-language name shown in the product. */
  label: string;
  /** Stable capability ID advertised by a tested Project connection. */
  capability: string;
  required: boolean;
};

export function toProjectConnectionScope(
  scope: AgentProjectScope,
): ProjectConnectionScope {
  return {
    relayUrl: scope.relayUrl,
    operatorPubkey: scope.operatorPubkey,
    projectAddress: scope.projectAddress,
  };
}

/**
 * Resolve the durable identity across the current one-repository read model
 * and the NIP-MP multi-repository Project model.
 */
export function durableProjectAddress(project: {
  projectAddress?: string;
  repoAddress?: string;
}): string {
  const address = project.projectAddress ?? project.repoAddress;
  if (!address) {
    throw new Error("This Project does not have a durable address.");
  }
  return address;
}
