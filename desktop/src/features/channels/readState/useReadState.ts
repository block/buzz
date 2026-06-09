import * as React from "react";
import { ReadStateManager } from "@/features/channels/readState/readStateManager";
import type { RelayClient } from "@/shared/api/relayClientSession";

const noopGetTimestamp = () => null;
const noopMarkRead = () => {};
const noopDrainAdvances = (): ReadonlySet<string> => new Set<string>();

/**
 * React hook that creates and manages a ReadStateManager instance.
 * Returns no-op functions when pubkey or relayClient are not available.
 */
export function useReadState(
  pubkey: string | undefined,
  relayClient: RelayClient | undefined,
) {
  const [readStateVersion, forceUpdate] = React.useReducer(
    (x: number) => x + 1,
    0,
  );
  const [initializedPubkey, setInitializedPubkey] = React.useState<
    string | null
  >(null);
  const managerRef = React.useRef<ReadStateManager | null>(null);

  // Create/destroy manager when pubkey becomes available/changes
  React.useEffect(() => {
    setInitializedPubkey(null);
    if (!pubkey || !relayClient) return;

    let isCancelled = false;
    const manager = new ReadStateManager(pubkey, relayClient);
    managerRef.current = manager;

    const unsubscribe = manager.subscribe(() => {
      forceUpdate();
    });

    void manager.initialize().finally(() => {
      if (!isCancelled) {
        setInitializedPubkey(pubkey);
      }
    });

    return () => {
      isCancelled = true;
      unsubscribe();
      manager.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayClient]);

  const getEffectiveTimestamp = React.useCallback(
    (contextId: string): number | null => {
      return managerRef.current?.getEffectiveTimestamp(contextId) ?? null;
    },
    [],
  );

  const markContextRead = React.useCallback(
    (contextId: string, unixTimestamp: number): void => {
      managerRef.current?.markContextRead(contextId, unixTimestamp);
    },
    [],
  );

  const seedContextRead = React.useCallback(
    (contextId: string, unixTimestamp: number): void => {
      managerRef.current?.seedContextRead(contextId, unixTimestamp);
    },
    [],
  );

  const drainSyncedAdvances = React.useCallback((): ReadonlySet<string> => {
    return managerRef.current?.drainSyncedAdvances() ?? new Set<string>();
  }, []);

  const isReady = Boolean(
    pubkey && relayClient && initializedPubkey === pubkey,
  );

  if (!pubkey || !relayClient) {
    return {
      getEffectiveTimestamp: noopGetTimestamp,
      isReady: false,
      markContextRead: noopMarkRead,
      seedContextRead: noopMarkRead,
      drainSyncedAdvances: noopDrainAdvances,
      readStateVersion: 0,
    };
  }

  return {
    getEffectiveTimestamp,
    isReady,
    markContextRead,
    seedContextRead,
    drainSyncedAdvances,
    readStateVersion,
  };
}
