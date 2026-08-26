import type { PresenceStatus } from "@/shared/api/types";

export type MentionScope = "channel" | "here";

export const MENTION_SCOPE_TAG: string;
export const MENTION_SCOPE_CHANNEL: "channel";
export const MENTION_SCOPE_HERE: "here";

export function detectMentionScope(
  content: string | null | undefined,
): MentionScope | null;

export function mentionScopeOf(
  tags: readonly (readonly string[])[] | null | undefined,
): MentionScope | null;

export function shouldNotifyForMentionScope(input: {
  scope: string;
  isAuthor?: boolean;
  isMember?: boolean;
  isMuted?: boolean;
  presence?: PresenceStatus;
  allowChannelMentionWhileMuted?: boolean;
}): boolean;
