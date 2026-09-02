/**
 * Roster-derived lookups cached on the roster array's identity.
 *
 * Channel rosters can be 10k+ members, and several render-path consumers
 * (timeline role badges, agent-flag merges, agent-session filters) each
 * derived their own Map/Set from the full roster on every recompute — every
 * live message re-ran the O(members) passes. React Query's structural sharing
 * keeps the roster array's identity stable across refetches that return
 * identical data, so a WeakMap keyed on that identity computes each
 * derivation once per distinct roster.
 */

import { normalizePubkey } from "@/shared/lib/pubkey";

type RosterMember = {
  pubkey: string;
  role: string;
  isAgent: boolean;
};

const roleMaps = new WeakMap<
  readonly RosterMember[],
  ReadonlyMap<string, string>
>();

/** Lowercased pubkey → channel role for the given roster. */
export function channelRoleMap<T extends RosterMember>(
  members: readonly T[],
): ReadonlyMap<string, string> {
  const cached = roleMaps.get(members);
  if (cached) return cached;
  const map = new Map<string, string>();
  for (const member of members) {
    map.set(member.pubkey.toLowerCase(), member.role);
  }
  roleMaps.set(members, map);
  return map;
}

const agentMemberLists = new WeakMap<
  readonly RosterMember[],
  readonly RosterMember[]
>();

/** Members that are agents: role `bot` or an explicit `isAgent` flag. */
export function channelAgentMembers<T extends RosterMember>(
  members: readonly T[],
): readonly T[] {
  const cached = agentMemberLists.get(members);
  if (cached) return cached as readonly T[];
  const agents = members.filter(
    (member) => member.role === "bot" || member.isAgent,
  );
  agentMemberLists.set(members, agents);
  return agents;
}

const memberPubkeySets = new WeakMap<
  readonly RosterMember[],
  ReadonlySet<string>
>();

/** Normalized pubkeys of every roster member. */
export function channelMemberPubkeySet(
  members: readonly RosterMember[],
): ReadonlySet<string> {
  const cached = memberPubkeySets.get(members);
  if (cached) return cached;
  const set = new Set(members.map((member) => normalizePubkey(member.pubkey)));
  memberPubkeySets.set(members, set);
  return set;
}

const botMemberPubkeySets = new WeakMap<
  readonly RosterMember[],
  ReadonlySet<string>
>();

/**
 * Normalized pubkeys of role-`bot` members only. Deliberately narrower than
 * {@link channelAgentMembers}: the agent-session filter treats the membership
 * role as authoritative and ignores profile-derived agent flags.
 */
export function channelBotMemberPubkeySet(
  members: readonly RosterMember[],
): ReadonlySet<string> {
  const cached = botMemberPubkeySets.get(members);
  if (cached) return cached;
  const set = new Set(
    members
      .filter((member) => member.role === "bot")
      .map((member) => normalizePubkey(member.pubkey)),
  );
  botMemberPubkeySets.set(members, set);
  return set;
}
