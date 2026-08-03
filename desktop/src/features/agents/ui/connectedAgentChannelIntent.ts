export type ConnectedAgentMembershipResult = {
  added: string[];
  errors: Array<{
    pubkey: string;
    error: string;
  }>;
};

function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

/**
 * Interpret the relay's batch membership result for one connected agent.
 *
 * `addChannelMembers` is batch-shaped even when this UI writes one pubkey. Keep
 * the exact matching and error precedence in a pure seam so the connected path
 * cannot mistake another batch entry for this agent's outcome.
 */
export function connectedAgentMembershipAdded(
  agentPubkey: string,
  result: ConnectedAgentMembershipResult,
): boolean {
  const normalizedAgent = normalizePubkey(agentPubkey);
  const membershipError = result.errors.find(
    (error) => normalizePubkey(error.pubkey) === normalizedAgent,
  );
  if (membershipError) {
    throw new Error(membershipError.error);
  }

  return result.added.some(
    (pubkey) => normalizePubkey(pubkey) === normalizedAgent,
  );
}
