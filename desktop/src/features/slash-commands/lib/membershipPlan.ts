import { channelNamesMatch } from "@/features/channels/lib/canonicalChannelName";

export const ALREADY_IN_CHANNEL_ERROR = "Already in this channel";
export const NOT_IN_CHANNEL_ERROR = "Not in this channel";
export const USE_LEAVE_ERROR = "Use /leave to leave this channel";

type JoinableChannel = {
  id: string;
  name: string;
  channelType: string;
  isMember: boolean;
  archivedAt: string | null;
};

function normalized(pubkeys: readonly string[]): string[] {
  return [...new Set(pubkeys.map((pubkey) => pubkey.trim().toLowerCase()))];
}

/** Mentioned pubkeys that are not yet in the channel, lowercased and unique. */
export function planInvite(
  mentioned: readonly string[],
  memberPubkeys: readonly string[],
): string[] {
  const members = new Set(normalized(memberPubkeys));
  const invitees = normalized(mentioned).filter((key) => !members.has(key));
  if (invitees.length === 0) throw new Error(ALREADY_IN_CHANNEL_ERROR);
  return invitees;
}

/**
 * Mentioned pubkeys to remove. Removing yourself is a different action with
 * its own confirmation, so it points at `/leave` instead.
 */
export function planRemove(
  mentioned: readonly string[],
  memberPubkeys: readonly string[],
  selfPubkey: string | null | undefined,
): string[] {
  const self = selfPubkey?.trim().toLowerCase();
  const keys = normalized(mentioned);
  if (self && keys.includes(self)) throw new Error(USE_LEAVE_ERROR);
  const members = new Set(normalized(memberPubkeys));
  const targets = keys.filter((key) => members.has(key));
  if (targets.length === 0) throw new Error(NOT_IN_CHANNEL_ERROR);
  return targets;
}

export type InviteGroups<Agent> = {
  /** Local managed agents: membership plus a running harness. */
  attach: Agent[];
  humans: string[];
  bots: string[];
};

/**
 * Splits invitees by the role the member list's add path gives them: local
 * managed agents are attached, other agents join as `bot`, everyone else as
 * `member`.
 */
export function groupInvitees<Agent>(
  pubkeys: readonly string[],
  localManagedAgents: ReadonlyMap<string, Agent>,
  isAgent: (pubkey: string) => boolean,
): InviteGroups<Agent> {
  const groups: InviteGroups<Agent> = { attach: [], humans: [], bots: [] };
  for (const pubkey of pubkeys) {
    const managed = localManagedAgents.get(pubkey);
    if (managed) groups.attach.push(managed);
    else if (isAgent(pubkey)) groups.bots.push(pubkey);
    else groups.humans.push(pubkey);
  }
  return groups;
}

/** Names written as `#name` in `args`, in order, without the `#`. */
export function parseChannelRefNames(args: string): string[] {
  return [...args.matchAll(/(?:^|[\s([{])#([\w-]+)/g)].map((match) => match[1]);
}

/** The channel a `/join` should act on: a joined one first, then an unarchived one. */
export function findChannelByName<Channel extends JoinableChannel>(
  name: string,
  channels: readonly Channel[],
): Channel | undefined {
  const matches = channels.filter(
    (channel) =>
      channel.channelType !== "dm" && channelNamesMatch(channel.name, name),
  );
  return (
    matches.find((channel) => channel.isMember) ??
    matches.find((channel) => channel.archivedAt === null) ??
    matches[0]
  );
}
