export const AGENT_ADDRESS_MENTION_MARKER: "agent-address";

export function buildAgentAddressMentionTags(
  addressedPubkeys: Iterable<string>,
  deliveredPubkeys: Iterable<string>,
): string[][];

export function getAgentAddressMentionPubkeys(
  tags: readonly (readonly string[])[] | null | undefined,
): string[];

export function isAgentAddressMentionTag(tag: readonly string[]): boolean;
