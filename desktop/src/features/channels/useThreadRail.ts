import * as React from "react";

import {
  DEFAULT_THREAD_RAIL_STORE,
  addThreadRailPin,
  readThreadRailStore,
  renameThreadRailPin,
  removeThreadRailPin,
  toggleThreadRailCollapsed,
  updateThreadRailExpandedReplyIds,
  updateThreadRailPinAnchor,
  writeThreadRailStore,
  type ThreadRailPin,
  type ThreadRailScope,
  type ThreadRailStore,
} from "./threadRailStorage";

type RailState = {
  scopeKey: string;
  store: ThreadRailStore;
};

const NO_SCOPE_KEY = "";

function scopeKey(
  pubkey: string | null | undefined,
  relayUrl: string | null | undefined,
): string {
  return pubkey && relayUrl ? `${pubkey}\u0000${relayUrl}` : NO_SCOPE_KEY;
}

/** Local thread-rail state and mutations for one pubkey-and-relay scope. */
export type ThreadRail = {
  pins: ThreadRailPin[];
  collapsed: boolean;
  isScoped: boolean;
  pin: (pin: ThreadRailPin) => void;
  rename: (
    pin: Pick<ThreadRailPin, "channelId" | "rootId">,
    customTitle: string,
  ) => void;
  unpin: (pin: Pick<ThreadRailPin, "channelId" | "rootId">) => void;
  updateAnchor: (
    pin: Pick<ThreadRailPin, "channelId" | "rootId">,
    returnAnchorId: string,
  ) => void;
  updateExpandedReplyIds: (
    pin: Pick<ThreadRailPin, "channelId" | "rootId">,
    expandedReplyIds: readonly string[],
  ) => void;
  toggleCollapsed: () => void;
};

/**
 * Keeps thread-rail preferences local to the current identity and relay.
 * A scope transition hides the previous scope immediately, then hydrates the
 * new scope from storage. Failed persistence never rolls back local state.
 */
export function useThreadRail(
  pubkey: string | null | undefined,
  relayUrl: string | null | undefined,
): ThreadRail {
  const currentScopeKey = scopeKey(pubkey, relayUrl);
  const scope = React.useMemo(
    () =>
      pubkey && relayUrl
        ? ({ pubkey, relayUrl } satisfies ThreadRailScope)
        : null,
    [pubkey, relayUrl],
  );
  const [railState, setRailState] = React.useState<RailState>({
    scopeKey: NO_SCOPE_KEY,
    store: DEFAULT_THREAD_RAIL_STORE,
  });

  React.useEffect(() => {
    setRailState({
      scopeKey: currentScopeKey,
      store: scope ? readThreadRailStore(scope) : DEFAULT_THREAD_RAIL_STORE,
    });
  }, [currentScopeKey, scope]);

  const updateStore = React.useCallback(
    (update: (store: ThreadRailStore) => ThreadRailStore) => {
      if (!scope) return;
      setRailState((current) => {
        if (current.scopeKey !== currentScopeKey) return current;
        const store = update(current.store);
        writeThreadRailStore(scope, store);
        return store === current.store ? current : { ...current, store };
      });
    },
    [currentScopeKey, scope],
  );

  const pin = React.useCallback(
    (pinToAdd: ThreadRailPin) =>
      updateStore((store) => addThreadRailPin(store, pinToAdd)),
    [updateStore],
  );
  const rename = React.useCallback(
    (
      pinToRename: Pick<ThreadRailPin, "channelId" | "rootId">,
      customTitle: string,
    ) =>
      updateStore((store) =>
        renameThreadRailPin(store, pinToRename, customTitle),
      ),
    [updateStore],
  );
  const unpin = React.useCallback(
    (pinToRemove: Pick<ThreadRailPin, "channelId" | "rootId">) =>
      updateStore((store) => removeThreadRailPin(store, pinToRemove)),
    [updateStore],
  );
  const updateAnchor = React.useCallback(
    (
      pinToUpdate: Pick<ThreadRailPin, "channelId" | "rootId">,
      returnAnchorId: string,
    ) =>
      updateStore((store) =>
        updateThreadRailPinAnchor(store, pinToUpdate, returnAnchorId),
      ),
    [updateStore],
  );
  const updateExpandedReplyIds = React.useCallback(
    (
      pinToUpdate: Pick<ThreadRailPin, "channelId" | "rootId">,
      expandedReplyIds: readonly string[],
    ) =>
      updateStore((store) =>
        updateThreadRailExpandedReplyIds(store, pinToUpdate, expandedReplyIds),
      ),
    [updateStore],
  );
  const toggleCollapsed = React.useCallback(
    () => updateStore(toggleThreadRailCollapsed),
    [updateStore],
  );

  const store =
    railState.scopeKey === currentScopeKey
      ? railState.store
      : DEFAULT_THREAD_RAIL_STORE;
  return React.useMemo(
    () => ({
      pins: store.pins,
      collapsed: store.collapsed,
      isScoped: scope !== null,
      pin,
      rename,
      unpin,
      updateAnchor,
      updateExpandedReplyIds,
      toggleCollapsed,
    }),
    [
      pin,
      rename,
      scope,
      store,
      toggleCollapsed,
      unpin,
      updateAnchor,
      updateExpandedReplyIds,
    ],
  );
}
