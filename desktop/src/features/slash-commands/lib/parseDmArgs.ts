import type { MentionRange } from "@/features/messages/lib/extractMentionPubkeys";

/** The relay rejects a DM with more than 8 other participants. */
export const MAX_DM_OTHER_PARTICIPANTS = 8;

/**
 * Splits `/dm` arguments into recipients and message. The recipients are the
 * mentions at the start; once any other text appears, later mentions belong
 * to the message as plain text.
 */
export function parseDmArgs(
  args: string,
  ranges: readonly MentionRange[],
): { recipients: string[]; message: string } {
  const recipients: string[] = [];
  let end = 0;
  for (const range of ranges) {
    if (args.slice(end, range.start).trim() !== "") break;
    recipients.push(range.pubkey);
    end = range.end;
  }
  return { recipients, message: args.slice(end).trim() };
}
