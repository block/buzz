import type { ChannelRole, PresenceStatus } from "@/shared/api/types";

export type MentionScope = "channel" | "here";

/** A channel admin's explicit override of the size-based default. */
export type MentionScopePolicy = "everyone" | "admins";

export const MENTION_SCOPE_TAG: string;
export const MENTION_SCOPE_CHANNEL: "channel";
export const MENTION_SCOPE_HERE: "here";
export const CHANNEL_MENTION_ADMIN_THRESHOLD: number;

export function detectMentionScope(
  content: string | null | undefined,
): MentionScope | null;

export function mentionScopeOf(
  tags: readonly (readonly string[])[] | null | undefined,
): MentionScope | null;

export function mentionScopeTag(scope: string): [string, string] | null;

export function canUseMentionScope(input: {
  scope: string;
  memberCount: number | undefined;
  role: ChannelRole | undefined;
  override?: MentionScopePolicy | null;
}): boolean;

export function shouldNotifyForMentionScope(input: {
  scope: string;
  isAuthor?: boolean;
  isMember?: boolean;
  isMuted?: boolean;
  presence?: PresenceStatus;
  allowChannelMentionWhileMuted?: boolean;
}): boolean;

export function resolveMentionAudience(input: {
  scope: string;
  members: readonly string[] | undefined;
  authorPubkey?: string | null;
  mutedBy?: ReadonlySet<string>;
  optedOutOfMutedChannelMentions?: ReadonlySet<string>;
  presenceByPubkey?: ReadonlyMap<string, PresenceStatus>;
}): string[];
