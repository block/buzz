import * as React from "react";

import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import { resolveRelayConnectivityCardVariant } from "@/shared/lib/relayConnectivityCard";
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
  const cardVariant = resolveRelayConnectivityCardVariant(
    errorMessage,
    relayUrl,
  );
  const isRelayConnectionStateDegraded =
    isRelayConnectionDegraded(relayConnectionState);
  const isRelayConnectionActuallyDegraded =
    hasRelayUnreachableError || isRelayConnectionStateDegraded;
  const isRelayConnectionConnected = relayConnectionState === "connected";
  const [isDismissed, setIsDismissed] = React.useState(false);
  const hasSuccess = React.useSyncExternalStore(
    subscribeRelayConnectivitySuccess,
    () => getRelayConnectivitySuccessSnapshot(relayUrl),
    () => false,
  );
  const [isWindowVisible, setIsWindowVisible] =
    React.useState(isDocumentVisible);
  const canShow = isRelayConnectionActuallyDegraded || hasSuccess;
  const show = canShow && !isDismissed;
  const wasProblemCardVisibleRef = React.useRef(false);
  const { isPending: isReconnectPending, reconnect } = useReconnectRelay();
  const [connectivityAction, setConnectivityAction] = React.useState<
    "relay-connection" | null
  >(null);
  const connectivityActionRef = React.useRef<"relay-connection" | null>(null);
  const connectivityFrameRef = React.useRef<number | null>(null);
  const connectivityTimeoutRef = React.useRef<number | null>(null);
  const isRelayReconnectPending =
    isReconnectPending || connectivityAction === "relay-connection";

  React.useEffect(() => {
    if (!isRelayConnectionActuallyDegraded && !hasSuccess) {
      setIsDismissed(false);
    }
  }, [hasSuccess, isRelayConnectionActuallyDegraded]);

  React.useEffect(() => {
    if (isRelayConnectionStateDegraded) {
      setRelayConnectivitySuccess(relayUrl, false);
      setIsDismissed(false);
    }
  }, [isRelayConnectionStateDegraded, relayUrl]);

  React.useEffect(() => {
    if (isRelayConnectionActuallyDegraded) {
      wasProblemCardVisibleRef.current = show && !hasSuccess;
      return;
    }

    if (wasProblemCardVisibleRef.current && isRelayConnectionConnected) {
      wasProblemCardVisibleRef.current = false;
      setRelayConnectivitySuccess(relayUrl, true);
    }
  }, [
    hasSuccess,
    relayUrl,
    show,
    isRelayConnectionActuallyDegraded,
    isRelayConnectionConnected,
  ]);

  React.useEffect(() => {
    if (!hasSuccess) {
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
  }, [hasSuccess, isWindowVisible, relayUrl]);

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
    cardVariant,
    hasRelayUnreachableError,
    isRelayConnectionSuccess: hasSuccess,
    isRelayReconnectPending,
    onDismissRelayConnectionCard: () => {
      setRelayConnectivitySuccess(relayUrl, false);
      setIsDismissed(true);
    },
    onReconnectRelay: handleReconnectRelay,
    showSidebarRelayConnectionCard: show,
  };
}
