import { CloudOff, RefreshCw } from "lucide-react";

import {
  SidebarCompactActionCard,
  type SidebarActionCardSurface,
} from "@/shared/ui/sidebar-action-card";
import { Spinner } from "@/shared/ui/spinner";

type SidebarRelayConnectionCardProps = {
  isActionDisabled?: boolean;
  actionTestId?: string;
  className?: string;
  isConnected?: boolean;
  isReconnectPending: boolean;
  isWaitingOnReconnectHook?: boolean;
  onDismiss?: () => void;
  onReconnect: () => void;
  transport?: "lan" | "public" | null;
  surface?: SidebarActionCardSurface;
  testId?: string;
};

export function SidebarRelayConnectionCard({
  actionTestId,
  className,
  isActionDisabled = false,
  isConnected = false,
  isReconnectPending,
  isWaitingOnReconnectHook = false,
  onDismiss,
  onReconnect,
  transport,
  surface,
}: SidebarRelayConnectionCardProps) {
  return (
    <SidebarRelayConnectionCompactCard
      actionTestId={actionTestId ?? "sidebar-reconnect"}
      className={className}
      isActionDisabled={isActionDisabled}
      isConnected={isConnected}
      isReconnectPending={isReconnectPending}
      isWaitingOnReconnectHook={isWaitingOnReconnectHook}
      onDismiss={onDismiss}
      onReconnect={onReconnect}
      transport={transport}
      surface={surface}
      testId="sidebar-relay-unreachable"
    />
  );
}

export function SidebarRelayConnectionCompactCard({
  actionTestId,
  className,
  isActionDisabled = false,
  isConnected = false,
  isReconnectPending,
  isWaitingOnReconnectHook = false,
  onDismiss,
  onReconnect,
  transport,
  surface,
  testId = "sidebar-relay-unreachable-compact",
}: SidebarRelayConnectionCardProps) {
  const reconnectTitle = isWaitingOnReconnectHook
    ? "Waiting to reconnect"
    : "Connecting";
  const reconnectDescription = isWaitingOnReconnectHook
    ? "Complete any prompts opened by the reconnect helper to continue."
    : "Reconnecting";

  return (
    <SidebarCompactActionCard
      actionAriaLabel={isConnected ? "Refresh connection" : "Connect to relay"}
      actionDisabled={isActionDisabled || isReconnectPending}
      actionTestId={actionTestId}
      description={
        isConnected
          ? transport === "lan"
            ? "Connected via LAN"
            : transport === "public"
              ? "Connected via public relay"
              : undefined
          : isReconnectPending
            ? reconnectDescription
            : "Click to connect"
      }
      dismissLabel="Dismiss relay notification"
      iconKey={
        isConnected ? "connected" : isReconnectPending ? "pending" : "idle"
      }
      icon={
        isConnected ? (
          <RefreshCw aria-hidden="true" className="h-5 w-5" />
        ) : isReconnectPending ? (
          <Spinner aria-hidden="true" className="h-5 w-5 border-2" />
        ) : (
          <CloudOff aria-hidden="true" className="h-5 w-5" />
        )
      }
      className={className}
      onAction={onReconnect}
      onDismiss={onDismiss}
      role={isConnected ? "status" : "alert"}
      surface={surface}
      testId={testId}
      title={
        isConnected
          ? "Connected"
          : isReconnectPending
            ? reconnectTitle
            : "Can't reach the relay"
      }
      tone={isConnected ? "success" : "neutral"}
    />
  );
}
