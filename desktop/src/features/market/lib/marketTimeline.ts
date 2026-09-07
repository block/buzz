import * as React from "react";
import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";
import type {
  MarketBid,
  MarketProjection,
} from "@/features/market/lib/marketProtocol";
import { parseMarketEnvelope } from "@/features/market/lib/marketProtocol";
import type { TimelineMessage } from "@/features/messages/types";

export function isMarketProtocolMessage(
  message: Pick<TimelineMessage, "body">,
): boolean {
  return parseMarketEnvelope(message.body) !== null;
}

const MARKET_PRODUCT_IMAGE_LABEL = "Product image for this offer";

function isRedundantMarketImageMessage(
  message: Pick<TimelineMessage, "body" | "parentId">,
  projection: MarketProjection,
): boolean {
  const imageUrl = projection.contract.listing.imageUrl;
  if (!imageUrl || message.parentId != null) return false;
  return (
    message.body.trim() ===
    `${MARKET_PRODUCT_IMAGE_LABEL}\n![image](${imageUrl})`
  );
}

export function marketTimelineAnchor(
  messages: TimelineMessage[],
  isChannelCreated: (message: TimelineMessage) => boolean,
): TimelineMessage | null {
  return (
    messages.find(
      (message) => message.parentId == null && isChannelCreated(message),
    ) ?? null
  );
}

export function marketBidsAfterAnchor(
  bids: MarketBid[],
  anchor: Pick<TimelineMessage, "createdAt" | "id">,
): MarketBid[] {
  return bids.filter(
    (bid) =>
      bid.createdAt > anchor.createdAt ||
      (bid.createdAt === anchor.createdAt &&
        bid.eventId.localeCompare(anchor.id) > 0),
  );
}

/** Keep protocol state out of the market's main chat; negotiation stays in threads. */
export function selectMarketTimelineMessages(
  messages: TimelineMessage[],
  projection: MarketProjection | null,
): TimelineMessage[] {
  if (!projection) return messages;
  return messages.filter(
    (message) =>
      !isMarketProtocolMessage(message) &&
      !isRedundantMarketImageMessage(message, projection),
  );
}

export function useMarketTimelineMessages(messages: TimelineMessage[]) {
  const projection = useMarketChannel();
  return React.useMemo(
    () => selectMarketTimelineMessages(messages, projection),
    [messages, projection],
  );
}
