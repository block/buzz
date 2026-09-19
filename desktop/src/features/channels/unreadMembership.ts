import * as React from "react";
import type { ObservedUnreadMembershipSeed } from "@/shared/api/tauriObservedUnread";
import { makeRootIdStore } from "@/features/channels/unreadRootIdStore";

export const participationStore = makeRootIdStore(
  "buzz-thread-participation.v1",
);
export const authoredStore = makeRootIdStore("buzz-thread-authored.v1");
export const mentionedStore = makeRootIdStore("buzz-thread-mentioned.v1");
export const mutedStore = makeRootIdStore("buzz-thread-muted.v1");

const EMPTY_IDS: ReadonlySet<string> = new Set();

/** Build the native membership seed once per identity/relay scope. */
export function useObservedUnreadMembershipSeed(
  scope: string,
  pubkey: string | undefined,
  followedRootIds: ReadonlySet<string> | undefined,
  mutedChannelIds: ReadonlySet<string>,
): ObservedUnreadMembershipSeed {
  const cached = React.useRef<{
    scope: string;
    value: ObservedUnreadMembershipSeed;
  } | null>(null);
  if (cached.current?.scope !== scope) {
    const read = (store: typeof participationStore) =>
      pubkey ? store.read(pubkey) : EMPTY_IDS;
    cached.current = {
      scope,
      value: {
        participatedRootIds: [...read(participationStore)],
        authoredRootIds: [...read(authoredStore)],
        mentionedRootIds: [...read(mentionedStore)],
        followedRootIds: [...(followedRootIds ?? EMPTY_IDS)],
        mutedRootIds: [...read(mutedStore)],
        mutedChannelIds: [...mutedChannelIds],
      },
    };
  }
  return cached.current.value;
}
