import { Check, CloudOff } from "lucide-react";

import {
  SidebarCompactActionCard,
  type SidebarActionCardSurface,
} from "@/shared/ui/sidebar-action-card";
import { Spinner } from "@/shared/ui/spinner";

type SidebarRelayConnectionCardProps = {
  isActionDisabled?: boolean;
  actionTestId?: string;
  className?: string;
  /**
   * The client's own backoff loop is already retrying — no user action is
   * needed. Distinct from `isReconnectPending`, which tracks a *manual*
   * reconnect the user asked for.
   */
  isAutoReconnecting?: boolean;
  isConnected?: boolean;
  isReconnectPending: boolean;
  isWaitingOnReconnectHook?: boolean;
  onDismiss?: () => void;
  onReconnect: () => void;
  surface?: SidebarActionCardSurface;
  testId?: string;
};

export function SidebarRelayConnectionCard({
  actionTestId,
  className,
  isActionDisabled = false,
  isAutoReconnecting = false,
  isConnected = false,
  isReconnectPending,
  isWaitingOnReconnectHook = false,
  onDismiss,
  onReconnect,
  surface,
}: SidebarRelayConnectionCardProps) {
  return (
    <SidebarRelayConnectionCompactCard
      actionTestId={actionTestId ?? "sidebar-reconnect"}
      className={className}
      isActionDisabled={isActionDisabled}
      isAutoReconnecting={isAutoReconnecting}
      isConnected={isConnected}
      isReconnectPending={isReconnectPending}
      isWaitingOnReconnectHook={isWaitingOnReconnectHook}
      onDismiss={onDismiss}
      onReconnect={onReconnect}
      surface={surface}
      testId="sidebar-relay-unreachable"
    />
  );
}

export function SidebarRelayConnectionCompactCard({
  actionTestId,
  className,
  isActionDisabled = false,
  isAutoReconnecting = false,
  isConnected = false,
  isReconnectPending,
  isWaitingOnReconnectHook = false,
  onDismiss,
  onReconnect,
  surface,
  testId = "sidebar-relay-unreachable-compact",
}: SidebarRelayConnectionCardProps) {
  const reconnectTitle = isWaitingOnReconnectHook
    ? "Waiting to reconnect"
    : "Connecting";
  const reconnectDescription = isWaitingOnReconnectHook
    ? "Complete any prompts opened by the reconnect helper to continue."
    : "Reconnecting";
  // A manual reconnect the user asked for outranks the background loop.
  const isRetryingWithoutUser = isAutoReconnecting && !isReconnectPending;
  const isBusy = isReconnectPending || isRetryingWithoutUser;

  return (
    <SidebarCompactActionCard
      actionAriaLabel={isConnected ? "Connected" : "Connect to relay"}
      // The background loop still yields the button: a user who does not want
      // to wait out the backoff can force an attempt now.
      actionDisabled={isActionDisabled || isReconnectPending || isConnected}
      actionTestId={actionTestId}
      description={
        isConnected
          ? undefined
          : isReconnectPending
            ? reconnectDescription
            : isRetryingWithoutUser
              ? "Trying to restore the connection"
              : "Click to connect"
      }
      dismissLabel="Dismiss relay notification"
      iconKey={isConnected ? "connected" : isBusy ? "pending" : "idle"}
      icon={
        isConnected ? (
          <Check aria-hidden="true" className="h-5 w-5" />
        ) : isBusy ? (
          <Spinner aria-hidden="true" className="h-5 w-5 border-2" />
        ) : (
          <CloudOff aria-hidden="true" className="h-5 w-5" />
        )
      }
      className={className}
      onAction={onReconnect}
      onDismiss={onDismiss}
      // Recovering on its own is a status, not an alarm — only escalate to
      // `alert` once the connection needs the user.
      role={isConnected || isRetryingWithoutUser ? "status" : "alert"}
      surface={surface}
      testId={testId}
      title={
        isConnected
          ? "Connected"
          : isReconnectPending
            ? reconnectTitle
            : isRetryingWithoutUser
              ? "Reconnecting"
              : "Can't reach the relay"
      }
      tone={isConnected ? "success" : "neutral"}
    />
  );
}
