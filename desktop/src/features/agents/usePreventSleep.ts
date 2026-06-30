import * as React from "react";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  getAgentObserverSnapshot,
  subscribeAgentObserverStore,
  useManagedAgentObserverBridge,
} from "@/features/agents/observerRelayStore";
import { createPreventSleepActivityTracker } from "@/features/agents/preventSleepActivity";
import { setPreventSleepActive } from "@/shared/api/tauri";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { listen } from "@tauri-apps/api/event";

// Intentionally not scoped per-pubkey — multi-user desktop is rare and the
// setting applies to the machine's sleep behavior regardless of account.
const STORAGE_KEY = "buzz-prevent-sleep";

function readPreference(): boolean {
  return window.localStorage.getItem(STORAGE_KEY) === "true";
}

function writePreference(enabled: boolean) {
  window.localStorage.setItem(STORAGE_KEY, String(enabled));
}

interface PreventSleepValue {
  enabled: boolean;
  setEnabled: (value: boolean) => void;
  active: boolean;
  hasRunningAgents: boolean;
  expired: boolean;
  clearExpired: () => void;
}

const PreventSleepContext = React.createContext<PreventSleepValue | null>(null);

export function PreventSleepProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const value = usePreventSleepInternal();
  return React.createElement(PreventSleepContext.Provider, { value }, children);
}

export function usePreventSleepContext(): PreventSleepValue {
  const ctx = React.useContext(PreventSleepContext);
  if (!ctx) {
    throw new Error(
      "usePreventSleepContext must be used within a PreventSleepProvider",
    );
  }
  return ctx;
}

function usePreventSleepInternal() {
  const [enabled, setEnabledState] = React.useState(readPreference);
  const { data: agents } = useManagedAgentsQuery();

  // Only local "running" agents need sleep prevention. Remote "deployed"
  // agents run on provider infrastructure and are unaffected by local sleep.
  const runningAgentPubkeys = React.useMemo(
    () =>
      (agents ?? [])
        .filter((agent) => agent.status === "running")
        .map((agent) => normalizePubkey(agent.pubkey))
        .sort(),
    [agents],
  );

  const runningAgentPubkeyKey = runningAgentPubkeys.join(",");
  const runningObserverAgents = React.useMemo(
    () =>
      enabled && runningAgentPubkeyKey
        ? runningAgentPubkeyKey.split(",").map((pubkey) => ({
            pubkey,
            status: "running" as const,
          }))
        : [],
    [enabled, runningAgentPubkeyKey],
  );

  useManagedAgentObserverBridge(runningObserverAgents);

  const hasRunningAgents = runningAgentPubkeys.length > 0;

  const [expired, setExpired] = React.useState(false);

  const active = enabled && hasRunningAgents && !expired;

  const setEnabled = React.useCallback((value: boolean) => {
    writePreference(value);
    setEnabledState(value);
  }, []);

  React.useEffect(() => {
    void setPreventSleepActive(active);
  }, [active]);
  React.useEffect(() => {
    const unlisten = listen("prevent-sleep-expired", () => {
      setExpired(true);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  React.useEffect(() => {
    if (!enabled || !runningAgentPubkeyKey) return;

    const observedPubkeys = runningAgentPubkeyKey.split(",");
    const tracker = createPreventSleepActivityTracker();
    const observeActivity = () => {
      const hasNewActivity = tracker.observe(
        observedPubkeys.map((pubkey) => ({
          pubkey,
          events: getAgentObserverSnapshot(pubkey, true).events,
        })),
      );
      if (!hasNewActivity) return;

      if (expired) {
        setExpired(false);
      }
      void setPreventSleepActive(true);
    };

    observeActivity();
    return subscribeAgentObserverStore(observeActivity);
  }, [enabled, expired, runningAgentPubkeyKey]);

  return {
    enabled,
    setEnabled,
    active,
    hasRunningAgents,
    expired,
    clearExpired: () => setExpired(false),
  };
}
