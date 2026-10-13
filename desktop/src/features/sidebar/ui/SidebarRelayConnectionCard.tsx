import { Check, CloudOff } from "lucide-react";

import {
  SidebarCompactActionCard,
  type SidebarActionCardSurface,
} from "@/shared/ui/sidebar-action-card";
import { Spinner } from "@/shared/ui/spinner";

// Web: the relay session retries on its own forever (backoff capped at 30 s), so the card never asks for a click — it just
// says it is reconnecting, then "Back online"; tapping is an optional nudge.
const IS_WEB = import.meta.env?.VITE_BUZZ_PLATFORM === "web";

type SidebarRelayConnectionCardProps = {
  isActionDisabled?: boolean;
  actionTestId?: string;
  className?: string;
  dismissClassName?: string;
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
  dismissClassName,
  isActionDisabled = false,
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

  if (IS_WEB) {
    return (
      <SidebarCompactActionCard
        actionAriaLabel={isConnected ? "Back online" : "Reconnect now"}
        actionDisabled={isActionDisabled || isReconnectPending || isConnected}
        actionTestId={actionTestId}
        description={isConnected ? undefined : isWaitingOnReconnectHook ? reconnectDescription : "Your messages will catch up automatically"}
        dismissClassName={dismissClassName}
        dismissLabel="Hide"
        iconKey={isConnected ? "connected" : "pending"}
        icon={
          isConnected ? (
            <Check aria-hidden="true" className="h-5 w-5" />
          ) : (
            <Spinner aria-hidden="true" className="h-5 w-5 border-2" />
          )
        }
        className={className}
        onAction={onReconnect}
        onDismiss={onDismiss}
        role="status"
        surface={surface}
        testId={testId}
        title={isConnected ? "Back online" : isWaitingOnReconnectHook ? reconnectTitle : "Reconnecting to relay…"}
        tone={isConnected ? "success" : "neutral"}
      />
    );
  }

  return (
    <SidebarCompactActionCard
      actionAriaLabel={isConnected ? "Connected" : "Connect to relay"}
      actionDisabled={isActionDisabled || isReconnectPending || isConnected}
      actionTestId={actionTestId}
      description={
        isConnected
          ? undefined
          : isReconnectPending
            ? reconnectDescription
            : "Click to connect"
      }
      dismissClassName={dismissClassName}
      dismissLabel="Dismiss relay notification"
      iconKey={
        isConnected ? "connected" : isReconnectPending ? "pending" : "idle"
      }
      icon={
        isConnected ? (
          <Check aria-hidden="true" className="h-5 w-5" />
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
