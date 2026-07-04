import * as React from "react";

import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import { isRelayUnreachableError } from "@/shared/lib/relayError";

const SIDEBAR_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS = 6_000;
const DEFAULT_RELAY_SUCCESS_KEY = "__default-relay__";

let relayConnectivitySuccessKey: string | null = null;
const relayConnectivitySuccessListeners = new Set<() => void>();

function relaySuccessKey(relayUrl: string | null | undefined) {
  return relayUrl ?? DEFAULT_RELAY_SUCCESS_KEY;
}

function subscribeRelayConnectivitySuccess(listener: () => void) {
  relayConnectivitySuccessListeners.add(listener);
  return () => relayConnectivitySuccessListeners.delete(listener);
}

function getRelayConnectivitySuccessSnapshot(
  relayUrl: string | null | undefined,
) {
  return relayConnectivitySuccessKey === relaySuccessKey(relayUrl);
}

function setRelayConnectivitySuccess(
  relayUrl: string | null | undefined,
  next: boolean,
) {
  const nextKey = next ? relaySuccessKey(relayUrl) : null;
  if (relayConnectivitySuccessKey === nextKey) {
    return;
  }

  // Don't let one workspace clear another workspace's success state.
  if (!next && relayConnectivitySuccessKey !== relaySuccessKey(relayUrl)) {
    return;
  }

  relayConnectivitySuccessKey = nextKey;
  for (const listener of relayConnectivitySuccessListeners) {
    listener();
  }
}

export function resetSidebarRelayConnectionCardState() {
  if (relayConnectivitySuccessKey === null) {
    return;
  }

  relayConnectivitySuccessKey = null;
  for (const listener of relayConnectivitySuccessListeners) {
    listener();
  }
}

function isDocumentVisible() {
  return document.visibilityState === "visible";
}

export function useSidebarRelayConnectionCard(
  errorMessage?: string,
  relayUrl?: string | null,
) {
  const relayConnectionState = useRelayConnection();
  const hasRelayUnreachableError = errorMessage
    ? isRelayUnreachableError(errorMessage)
    : false;
  // True when the error is an application-level issue (e.g. auth rejection)
  // rather than a network-level relay-unreachable error. In this case, the
  // disconnected state should NOT trigger the reconnect card — the app shows
  // a dedicated error path instead.
  const hasNonUnreachableError =
    Boolean(errorMessage) && !hasRelayUnreachableError;
  const isRelayConnectionStateDegraded =
    relayConnectionState === "reconnecting" ||
    relayConnectionState === "stalled" ||
    (relayConnectionState === "disconnected" && !hasNonUnreachableError);
  const isRelayConnectionConnected = relayConnectionState === "connected";
  const isRelayConnectionDisconnected = relayConnectionState === "disconnected";
  const [isDismissed, setIsDismissed] = React.useState(false);
  const hasSuccess = React.useSyncExternalStore(
    subscribeRelayConnectivitySuccess,
    () => getRelayConnectivitySuccessSnapshot(relayUrl),
    () => false,
  );
  const [isWindowVisible, setIsWindowVisible] =
    React.useState(isDocumentVisible);
  const hasActiveRelayUnreachableError =
    hasRelayUnreachableError && !hasSuccess && !isRelayConnectionConnected;
  const isRelayConnectionActuallyDegraded =
    hasActiveRelayUnreachableError || isRelayConnectionStateDegraded;
  const isRelayConnectionSuccess = hasSuccess && isRelayConnectionConnected;
  const canShow = isRelayConnectionActuallyDegraded || isRelayConnectionSuccess;
  const show = canShow && !isDismissed;
  const wasProblemCardVisibleRef = React.useRef(false);
  const {
    isPending: isReconnectPending,
    isWaitingOnReconnectHook,
    reconnect,
  } = useReconnectRelay();
  const [connectivityAction, setConnectivityAction] = React.useState<
    "relay-connection" | null
  >(null);
  const connectivityActionRef = React.useRef<"relay-connection" | null>(null);
  const connectivityFrameRef = React.useRef<number | null>(null);
  const connectivityTimeoutRef = React.useRef<number | null>(null);
  const isRelayReconnectPending =
    isReconnectPending || connectivityAction === "relay-connection";

  React.useEffect(() => {
    if (!isRelayConnectionActuallyDegraded && !isRelayConnectionSuccess) {
      setIsDismissed(false);
    }
  }, [isRelayConnectionSuccess, isRelayConnectionActuallyDegraded]);

  React.useEffect(() => {
    if (isRelayConnectionStateDegraded || isRelayConnectionDisconnected) {
      setRelayConnectivitySuccess(relayUrl, false);
      setIsDismissed(false);
    }
  }, [isRelayConnectionDisconnected, isRelayConnectionStateDegraded, relayUrl]);

  React.useEffect(() => {
    if (isRelayConnectionActuallyDegraded) {
      wasProblemCardVisibleRef.current = show && !isRelayConnectionSuccess;
      return;
    }

    if (wasProblemCardVisibleRef.current && isRelayConnectionConnected) {
      wasProblemCardVisibleRef.current = false;
      setRelayConnectivitySuccess(relayUrl, true);
    }
  }, [
    isRelayConnectionSuccess,
    relayUrl,
    show,
    isRelayConnectionActuallyDegraded,
    isRelayConnectionConnected,
  ]);

  React.useEffect(() => {
    if (!isRelayConnectionSuccess) {
      return;
    }

    if (!isWindowVisible) {
      return;
    }

    const timeout = window.setTimeout(() => {
      setRelayConnectivitySuccess(relayUrl, false);
      setIsDismissed(true);
    }, SIDEBAR_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS);

    return () => window.clearTimeout(timeout);
  }, [isRelayConnectionSuccess, isWindowVisible, relayUrl]);

  React.useEffect(() => {
    const updateWindowVisible = () => setIsWindowVisible(isDocumentVisible());

    document.addEventListener("visibilitychange", updateWindowVisible);

    return () => {
      document.removeEventListener("visibilitychange", updateWindowVisible);
    };
  }, []);

  React.useEffect(() => {
    return () => {
      if (connectivityFrameRef.current !== null) {
        window.cancelAnimationFrame(connectivityFrameRef.current);
      }
      if (connectivityTimeoutRef.current !== null) {
        window.clearTimeout(connectivityTimeoutRef.current);
      }
      connectivityActionRef.current = null;
    };
  }, []);

  const startConnectivityAction = React.useCallback(
    (runAction: () => Promise<void>) => {
      if (connectivityActionRef.current !== null) {
        return;
      }

      connectivityActionRef.current = "relay-connection";
      setConnectivityAction("relay-connection");
      connectivityFrameRef.current = window.requestAnimationFrame(() => {
        connectivityFrameRef.current = null;
        connectivityTimeoutRef.current = window.setTimeout(() => {
          connectivityTimeoutRef.current = null;
          void Promise.resolve()
            .then(runAction)
            .catch((error) => {
              console.error("[AppSidebar] connectivity action failed:", error);
            })
            .finally(() => {
              connectivityActionRef.current = null;
              setConnectivityAction(null);
            });
        }, 0);
      });
    },
    [],
  );

  const handleReconnectRelay = React.useCallback(() => {
    startConnectivityAction(async () => {
      setRelayConnectivitySuccess(relayUrl, false);
      const didReconnect = await reconnect();
      if (didReconnect) {
        wasProblemCardVisibleRef.current = false;
        setIsDismissed(false);
        setRelayConnectivitySuccess(relayUrl, true);
      }
    });
  }, [reconnect, relayUrl, startConnectivityAction]);

  return {
    hasRelayUnreachableError,
    isRelayConnectionSuccess,
    isRelayReconnectPending,
    isWaitingOnReconnectHook,
    onDismissRelayConnectionCard: () => {
      setRelayConnectivitySuccess(relayUrl, false);
      setIsDismissed(true);
    },
    onReconnectRelay: handleReconnectRelay,
    showSidebarRelayConnectionCard: show,
  };
}
