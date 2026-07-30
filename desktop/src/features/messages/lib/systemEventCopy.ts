/**
 * Copy for channel system events (the "joined", "added by", "changed the
 * topic" captions in the message timeline).
 *
 * These live outside `SystemMessageRow` so the wording is a pure function of
 * the payload and can be asserted directly in tests. Only cases whose caption
 * is plain text belong here — cases that interpolate a profile link build their
 * JSX in the component.
 */

/** Curly quotes, so the caption matches the typography used elsewhere in chat. */
const OPEN_QUOTE = "“";
const CLOSE_QUOTE = "”";

export type ChannelTextField = "topic" | "purpose";

/**
 * Caption for a channel topic or purpose change.
 *
 * Bare "the topic" rather than "the channel topic": this row only ever renders
 * in a channel timeline, under that channel's own header, so naming the channel
 * again is redundant — and it keeps the cleared and changed captions on the same
 * noun instead of one saying "channel topic" and the other "topic".
 *
 * A blank value means the field was cleared: the relay reports a clear as a
 * `topic_changed` / `purpose_changed` event carrying an empty string, not as a
 * separate event type. Without this branch the timeline renders `changed the
 * topic to ""`, which reads like the topic was set to two quote marks.
 * Whitespace-only values are treated as cleared for the same reason.
 */
export function describeChannelTextFieldChange(
  field: ChannelTextField,
  value: string | null | undefined,
): string {
  const trimmed = value?.trim();
  if (!trimmed) {
    return `cleared the ${field}`;
  }
  return `changed the ${field} to ${OPEN_QUOTE}${trimmed}${CLOSE_QUOTE}`;
}

/**
 * Adjusts a resolved display name for use inside a sentence rather than in the
 * name slot at the top of a row — "added by you", "removed you from the channel".
 *
 * `resolveUserLabel` returns "You" for the current user, which is right standing
 * alone and wrong mid-phrase. Agent ownership already draws the same distinction
 * from the other side: `formatOwnerLabel` returns lowercase "you" because it is
 * only ever read as "managed by you".
 *
 * `isSelf` is the caller's pubkey comparison, not an inspection of `label`.
 * Matching on the string would also rewrite a different person whose display
 * name happens to be "You" — the label is user-controlled, identity is not.
 * Every name that isn't the reader's own is a proper noun and is returned
 * untouched.
 */
export function toInlineName(label: string, isSelf: boolean): string {
  return isSelf ? "you" : label;
}
