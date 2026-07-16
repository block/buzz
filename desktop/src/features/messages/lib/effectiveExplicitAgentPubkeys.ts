function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

function uniqueNormalizedPubkeys(pubkeys: Iterable<string>): string[] {
  return [...new Set([...pubkeys].map(normalizePubkey))].filter(Boolean);
}

export function filterEffectiveExplicitAgentPubkeys(
  explicitAgentPubkeys: Iterable<string>,
  effectiveMentionPubkeys: Iterable<string>,
): string[] {
  const effectivePubkeys = new Set(
    uniqueNormalizedPubkeys(effectiveMentionPubkeys),
  );
  return uniqueNormalizedPubkeys(explicitAgentPubkeys).filter((pubkey) =>
    effectivePubkeys.has(pubkey),
  );
}
