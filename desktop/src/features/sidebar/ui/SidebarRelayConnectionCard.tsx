import { Check, CloudOff, Loader2 } from "lucide-react";

import {
  SidebarCompactActionCard,
  type SidebarActionCardSurface,
} from "@/shared/ui/sidebar-action-card";

type SidebarRelayConnectionCardProps = {
  isActionDisabled?: boolean;
  actionTestId?: string;
  isConnected?: boolean;
  isReconnectPending: boolean;
  onDismiss?: () => void;
  onReconnect: () => void;
  surface?: SidebarActionCardSurface;
  testId?: string;
};

export function SidebarRelayConnectionCard({
  actionTestId,
  isActionDisabled = false,
  isConnected = false,
  isReconnectPending,
  onDismiss,
  onReconnect,
  surface,
}: SidebarRelayConnectionCardProps) {
  return (
    <SidebarRelayConnectionCompactCard
      actionTestId={actionTestId ?? "sidebar-reconnect"}
      isActionDisabled={isActionDisabled}
      isConnected={isConnected}
      isReconnectPending={isReconnectPending}
      onDismiss={onDismiss}
      onReconnect={onReconnect}
      surface={surface}
      testId="sidebar-relay-unreachable"
    />
  );
}

export function SidebarRelayConnectionCompactCard({
  actionTestId,
  isActionDisabled = false,
  isConnected = false,
  isReconnectPending,
  onDismiss,
  onReconnect,
  surface,
  testId = "sidebar-relay-unreachable-compact",
}: SidebarRelayConnectionCardProps) {
  return (
    <SidebarCompactActionCard
      actionAriaLabel={isConnected ? "Connected" : "Connect to relay"}
      actionDisabled={isActionDisabled || isReconnectPending || isConnected}
      actionTestId={actionTestId}
      description={
        isConnected
          ? undefined
          : isReconnectPending
            ? "Reconnecting"
            : "Click to connect"
      }
      dismissLabel="Dismiss relay notification"
      iconKey={
        isConnected ? "connected" : isReconnectPending ? "pending" : "idle"
      }
      icon={
        isConnected ? (
          <Check aria-hidden="true" className="h-5 w-5" />
        ) : isReconnectPending ? (
          <Loader2 aria-hidden="true" className="h-5 w-5 animate-spin" />
        ) : (
          <CloudOff aria-hidden="true" className="h-5 w-5" />
        )
      }
      onAction={onReconnect}
      onDismiss={onDismiss}
      role={isConnected ? "status" : "alert"}
      surface={surface}
      testId={testId}
      title={isConnected ? "Connected" : "Can't reach the relay"}
      tone={isConnected ? "success" : "neutral"}
    />
  );
}
