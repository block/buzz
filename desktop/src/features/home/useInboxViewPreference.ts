import * as React from "react";

import type { InboxFilter } from "@/features/home/lib/inbox";
import {
  DEFAULT_INBOX_VIEW_PREFERENCE,
  inboxViewPreferenceStorageKey,
  readInboxViewPreference,
  writeInboxViewPreference,
  type InboxViewPreference,
} from "./inboxViewPreference";

/** State and mutations exposed to the mounted Inbox surface. */
export type InboxViewPreferenceController = {
  filter: InboxFilter;
  setFilter: (filter: InboxFilter) => void;
  setUnreadOnly: (unreadOnly: boolean) => void;
  unreadOnly: boolean;
};

/**
 * Device-local Inbox view preferences, scoped by signed-in user and relay.
 * They survive navigation and restart without being sent to the relay.
 */
export function useInboxViewPreference(
  pubkey: string | undefined,
  relayUrl?: string,
): InboxViewPreferenceController {
  const [preference, setPreference] = React.useState<InboxViewPreference>(() =>
    pubkey
      ? readInboxViewPreference(pubkey, relayUrl)
      : DEFAULT_INBOX_VIEW_PREFERENCE,
  );

  React.useEffect(() => {
    setPreference(
      pubkey
        ? readInboxViewPreference(pubkey, relayUrl)
        : DEFAULT_INBOX_VIEW_PREFERENCE,
    );
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    if (!pubkey) return;
    const key = inboxViewPreferenceStorageKey(pubkey, relayUrl);
    const handler = (event: StorageEvent) => {
      if (event.key === key) {
        setPreference(readInboxViewPreference(pubkey, relayUrl));
      }
    };
    window.addEventListener("storage", handler);
    return () => window.removeEventListener("storage", handler);
  }, [pubkey, relayUrl]);

  const updatePreference = React.useCallback(
    (patch: Partial<Pick<InboxViewPreference, "filter" | "unreadOnly">>) => {
      if (!pubkey) return;
      setPreference((previous) => {
        const next = { ...previous, ...patch };
        return writeInboxViewPreference(pubkey, next, relayUrl)
          ? next
          : previous;
      });
    },
    [pubkey, relayUrl],
  );

  const setFilter = React.useCallback(
    (filter: InboxFilter) => updatePreference({ filter }),
    [updatePreference],
  );
  const setUnreadOnly = React.useCallback(
    (unreadOnly: boolean) => updatePreference({ unreadOnly }),
    [updatePreference],
  );

  return {
    filter: preference.filter,
    setFilter,
    setUnreadOnly,
    unreadOnly: preference.unreadOnly,
  };
}
