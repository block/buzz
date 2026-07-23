export const localAgentRelayAllowedQueryKey = (communityId: string | null) =>
  ["local-agent-relay-allowed", communityId ?? "none"] as const;
