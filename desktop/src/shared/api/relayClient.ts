import { RelayClient } from "@/shared/api/relayClientSession";

export const relayClient = new RelayClient();

/**
 * Notify the relay client which channel is currently visible in the UI.
 *
 * On reconnect, subscriptions for the visible channel are sent in the first
 * replay batch so the user sees their active channel recover before others
 * on degraded networks.
 *
 * Call with `null` when the user navigates away from a channel view.
 */
export function setVisibleChannel(id: string | null): void {
  relayClient.setVisibleChannelId(id);
}
