/** Pure helpers for Huddle screen-share policy (no LiveKit import). */

/** Derive LiveKit room name the same way the relay does. */
export function livekitRoomName(channelId: string): string {
  return `huddle-${channelId}`;
}

/** Share is blocked when another pubkey already holds the slot. */
export function isShareBlockedByOther(args: {
  selfPubkey: string | null | undefined;
  currentSharer: string | null | undefined;
}): boolean {
  if (!args.currentSharer) return false;
  if (!args.selfPubkey) return true;
  return args.currentSharer.toLowerCase() !== args.selfPubkey.toLowerCase();
}
