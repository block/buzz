/** Community bot names that local managed agents must not reuse. */
export const RESERVED_COMMUNITY_AGENT_NAMES = [
  "Captain",
  "Mo",
  "Stitch",
  "Quasar",
  "Korg",
] as const;

const RESERVED_LOOKUP = new Set(
  RESERVED_COMMUNITY_AGENT_NAMES.map((name) => name.toLocaleLowerCase()),
);

export function isReservedCommunityAgentName(name: string): boolean {
  return RESERVED_LOOKUP.has(name.trim().toLocaleLowerCase());
}

export function reservedCommunityAgentNameError(name: string): string {
  const trimmed = name.trim();
  return `"${trimmed}" is reserved for a Hula community bot. Pick a different local agent name (for example "${trimmed} Local").`;
}
