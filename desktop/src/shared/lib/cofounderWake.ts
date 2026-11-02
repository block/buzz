import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";

/**
 * The tag the relay puts on the message that wakes Scout in co-founder mode.
 * The message is for Scout's harness only: a person is never meant to read it.
 */
export const COFOUNDER_WAKE_TAG = "buzz:cofounder-wake";

/**
 * Whether this event is a co-founder wake, which no human surface may show: not
 * the conversation, Activity, unread counts, notifications or search.
 *
 * It must be a stream message carrying the wake tag AND be signed by the relay
 * itself, so a member cannot make their own message invisible by copying the
 * tag. `relaySelf` is the relay's NIP-11 `self` key. While it is still unknown
 * (`undefined`) or the relay advertises none (`null`) the author cannot be
 * checked, so a tagged message stays hidden: failing closed hides one message
 * from one startup window, failing open would show the owner a stranger's post.
 */
export function isCofounderWakeEvent(
  event: Pick<RelayEvent, "kind" | "pubkey" | "tags">,
  relaySelf: string | null | undefined,
): boolean {
  if (
    event.kind !== KIND_STREAM_MESSAGE &&
    event.kind !== KIND_STREAM_MESSAGE_V2
  ) {
    return false;
  }
  if (!event.tags.some((tag) => tag[0] === COFOUNDER_WAKE_TAG)) return false;
  if (!relaySelf) return true;
  return event.pubkey.toLowerCase() === relaySelf.toLowerCase();
}

/**
 * Drops the co-founder wakes from a list of events or feed items. The relay's
 * key is only read when a tagged item is present, so an ordinary list costs one
 * scan of the tags.
 */
export async function withoutCofounderWakes<
  T extends Pick<RelayEvent, "kind" | "pubkey" | "tags">,
>(items: T[], readRelaySelf: () => Promise<string | null>): Promise<T[]> {
  if (!items.some((item) => isCofounderWakeEvent(item, undefined))) {
    return items;
  }
  const relaySelf = await readRelaySelf().catch(() => null);
  return items.filter((item) => !isCofounderWakeEvent(item, relaySelf));
}
