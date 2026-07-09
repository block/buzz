/**
 * React binding for the relay reconnect controller.
 *
 * Delegates all reconnect logic to the module-level `relayReconnectController`
 * singleton so that all mounted hook instances (banner + sidebar card) share
 * a single in-flight state. Deliberately uses `preconnect()` rather than the
 * full `reconnectWorkspace()` path to avoid unmounting the React tree and
 * clearing draft state.
 *
 * See `relayReconnectController.ts` for the three-phase strategy details.
 */

import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import { relayClient } from "@/shared/api/relayClient";
import { isRelayDependentQuery } from "@/shared/api/relayQueryInvalidation";
import { relayReconnectController } from "@/shared/api/relayReconnectController";

function buildDeps(onSuccess: () => void, onBackstop: () => void) {
  return {
    preconnect: () => relayClient.preconnect(),
    hookConfigured: () => invoke<boolean>("relay_reconnect_hook_configured"),
    runHook: () => invoke<void>("relay_reconnect_hook"),
    subscribeToConnectionState: (listener: (state: string) => void) =>
      relayClient.subscribeToConnectionState(listener),
    onSuccess,
    onBackstop,
    setTimeout: window.setTimeout.bind(window),
    clearTimeout: window.clearTimeout.bind(window),
    setInterval: window.setInterval.bind(window),
    clearInterval: window.clearInterval.bind(window),
  };
}

export function useReconnectRelay(): {
  reconnect: () => Promise<boolean>;
  isPending: boolean;
  isWaitingOnReconnectHook: boolean;
} {
  const queryClient = useQueryClient();

  const [controllerState, setControllerState] = React.useState(() =>
    relayReconnectController.getState(),
  );

  // Subscribe to controller state changes for the lifetime of this component.
  // Multiple hook instances receive the same state from the shared singleton.
  React.useEffect(() => {
    return relayReconnectController.subscribe(setControllerState);
  }, []);

  // Stable mutable refs for callbacks — updated every render so stale closures
  // are never captured, but the reconnect callback itself never changes identity.
  const onSuccessRef = React.useRef<(() => void) | null>(null);
  const onBackstopRef = React.useRef<(() => void) | null>(null);

  onSuccessRef.current = React.useCallback(() => {
    // Defer query invalidation so callers render the recovered state first.
    window.setTimeout(() => {
      void queryClient
        .invalidateQueries({ predicate: isRelayDependentQuery })
        .catch((err) => {
          console.error(
            "[useReconnectRelay] failed to refresh queries after reconnect:",
            err,
          );
        });
    }, 0);
  }, [queryClient]);

  onBackstopRef.current = React.useCallback(() => {
    toast("Still trying to reconnect — check your network.");
  }, []);

  const reconnect = React.useCallback(async () => {
    const deps = buildDeps(
      () => onSuccessRef.current?.(),
      () => onBackstopRef.current?.(),
    );
    return relayReconnectController.start(deps);
  }, []);

  return {
    reconnect,
    isPending: controllerState.isPending,
    isWaitingOnReconnectHook: controllerState.isWaitingOnReconnectHook,
  };
}
