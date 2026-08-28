import { RelayClient } from "@/shared/api/relayClientSession";
import { createVisibleChannelOwnership } from "@/shared/api/visibleChannelOwnership";

export const relayClient = new RelayClient();
const visibleChannelOwnership = createVisibleChannelOwnership((channelId) =>
  relayClient.setVisibleChannelId(channelId),
);

/** Keep reconnect priority on the newest surface without letting an older
 * surface's cleanup clear a channel that remains visible elsewhere. */
export function acquireVisibleChannel(id: string): () => void {
  return visibleChannelOwnership.acquire(id);
}
