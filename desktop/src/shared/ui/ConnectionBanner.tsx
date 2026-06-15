import { WifiOff } from "lucide-react";

import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import type { ConnectionState } from "@/shared/api/relayClientShared";

const COPY: Partial<Record<ConnectionState, string>> = {
  reconnecting: "Reconnecting to relay…",
  stalled: "Connection lost — relay is not responding.",
  disconnected: "Disconnected from relay.",
};

/**
 * Thin warning strip surfaced when the relay connection is degraded.
 *
 * Renders null while the connection is healthy so it takes up no layout space.
 * The strip auto-disappears once the state transitions back to "connected" —
 * no success toast needed.
 */
export function ConnectionBanner() {
  const state = useRelayConnection();
  const { isPending, reconnect } = useReconnectRelay();

  if (!isRelayConnectionDegraded(state)) return null;

  const message = COPY[state] ?? "Connection issue detected.";

  return (
    <div
      className="flex shrink-0 items-center gap-2 border-b border-warning/30 bg-warning/5 px-3 py-2 text-xs"
      data-testid="connection-banner"
      role="alert"
    >
      <WifiOff className="h-3 w-3 shrink-0 text-warning" />
      <span className="flex-1 text-muted-foreground">{message}</span>
      <button
        className="font-medium text-warning hover:underline disabled:opacity-50"
        data-testid="connection-banner-reconnect"
        disabled={isPending}
        onClick={reconnect}
        type="button"
      >
        {isPending ? "Reconnecting…" : "Reconnect"}
      </button>
    </div>
  );
}
