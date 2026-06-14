import * as React from "react";

import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import { isRelayUnreachableError } from "@/shared/lib/relayError";

const SIDEBAR_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS = 2_500;

export function useSidebarRelayConnectionCard(errorMessage?: string) {
  const relayConnectionState = useRelayConnection();
  const hasRelayUnreachableError = errorMessage
    ? isRelayUnreachableError(errorMessage)
    : false;
  const isRelayConnectionActuallyDegraded =
    hasRelayUnreachableError || isRelayConnectionDegraded(relayConnectionState);
  const isRelayConnectionConnected = relayConnectionState === "connected";
  const [isDismissed, setIsDismissed] = React.useState(false);
  const [hasSuccess, setHasSuccess] = React.useState(false);
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
    if (isRelayConnectionActuallyDegraded) {
      setHasSuccess(false);
      setIsDismissed(false);
    }
  }, [isRelayConnectionActuallyDegraded]);

  React.useEffect(() => {
    if (isRelayConnectionActuallyDegraded) {
      wasProblemCardVisibleRef.current = show && !hasSuccess;
      return;
    }

    if (wasProblemCardVisibleRef.current && isRelayConnectionConnected) {
      wasProblemCardVisibleRef.current = false;
      setHasSuccess(true);
    }
  }, [
    hasSuccess,
    show,
    isRelayConnectionActuallyDegraded,
    isRelayConnectionConnected,
  ]);

  React.useEffect(() => {
    if (!hasSuccess) {
      return;
    }

    const timeout = window.setTimeout(() => {
      setHasSuccess(false);
      setIsDismissed(true);
    }, SIDEBAR_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS);

    return () => window.clearTimeout(timeout);
  }, [hasSuccess]);

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
      setHasSuccess(false);
      const didReconnect = await reconnect();
      if (didReconnect) {
        setHasSuccess(true);
      }
    });
  }, [reconnect, startConnectivityAction]);

  return {
    hasRelayUnreachableError,
    isRelayConnectionSuccess: hasSuccess,
    isRelayReconnectPending,
    onDismissRelayConnectionCard: () => setIsDismissed(true),
    onReconnectRelay: handleReconnectRelay,
    showSidebarRelayConnectionCard: show,
  };
}
