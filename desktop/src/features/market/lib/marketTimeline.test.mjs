import assert from "node:assert/strict";
import test from "node:test";

import {
  isMarketProtocolMessage,
  marketBidsAfterAnchor,
  marketTimelineAnchor,
  selectMarketTimelineMessages,
} from "./marketTimeline.ts";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { isChannelCreatedSystemMessage } from "@/features/channels/ui/ChannelPane.helpers";
import { buildIndependentThreadPanel } from "@/features/messages/lib/independentThreadPanel";

const PUBKEY = "f".repeat(64);
const CHANNEL = "123e4567-e89b-12d3-a456-426614174000";
const envelope = (type) =>
  JSON.stringify({
    protocol: "buzz-market/v0",
    type,
    channelId: CHANNEL,
    ...(type === "contract"
      ? {
          version: 1,
          listing: {
            actorName: "Seller",
            direction: "offer",
            mechanism: "fixed",
            title: "Report",
            summary: "Deliver a report",
            quantity: 1,
            priceSats: 50,
          },
        }
      : {
          listingEventId: "1".repeat(64),
          actorName: "Buyer",
          quantity: 1,
          amountSats: 50,
          message: "I bid",
        }),
  });
const message = (id, body, parentId = null, rootId = null, fields = {}) => ({
  id,
  body,
  parentId,
  rootId,
  ...fields,
});

test("the market board anchors to channel creation, not above history", () => {
  const contract = message("1".repeat(64), envelope("contract"), null, null, {
    createdAt: 10,
  });
  const created = message(
    "2".repeat(64),
    JSON.stringify({ type: "channel_created" }),
    null,
    null,
    { createdAt: 11, kind: KIND_SYSTEM_MESSAGE },
  );
  const joined = message(
    "3".repeat(64),
    JSON.stringify({ type: "member_joined" }),
    null,
    null,
    { createdAt: 12, kind: KIND_SYSTEM_MESSAGE },
  );

  assert.equal(
    marketTimelineAnchor(
      [contract, created, joined],
      isChannelCreatedSystemMessage,
    )?.id,
    created.id,
  );
  assert.equal(isMarketProtocolMessage(contract), true);
  assert.equal(isMarketProtocolMessage(created), false);
  assert.deepEqual(
    marketBidsAfterAnchor(
      [
        { createdAt: 9, eventId: "9".repeat(64) },
        { createdAt: 11, eventId: "1".repeat(64) },
        { createdAt: 11, eventId: "3".repeat(64) },
        { createdAt: 12, eventId: "0".repeat(64) },
      ],
      created,
    ).map(({ eventId }) => eventId),
    ["3".repeat(64), "0".repeat(64)],
  );
});

test("market timeline hides protocol events but leaves negotiation in its bid thread", () => {
  const contract = message("1".repeat(64), envelope("contract"));
  const bid = message("2".repeat(64), envelope("response"));
  const negotiation = message(
    "3".repeat(64),
    "Can you deliver sooner?",
    bid.id,
    bid.id,
  );
  const nestedProtocol = message(
    "4".repeat(64),
    envelope("response"),
    bid.id,
    bid.id,
  );
  const imageUrl = "https://relay.example/media/product.png";
  const productImage = message(
    "5".repeat(64),
    `Product image for this offer\n![image](${imageUrl})`,
  );
  const imageReply = message(
    "6".repeat(64),
    `Product image for this offer\n![image](${imageUrl})`,
    bid.id,
    bid.id,
  );
  const ordinary = message("7".repeat(64), "General market note");
  const projection = {
    listingEventId: contract.id,
    contract: { listing: { imageUrl } },
    bids: [{ eventId: bid.id }],
  };

  assert.deepEqual(
    selectMarketTimelineMessages(
      [
        contract,
        bid,
        negotiation,
        nestedProtocol,
        productImage,
        imageReply,
        ordinary,
      ],
      projection,
    ).map(({ id }) => id),
    [negotiation.id, imageReply.id, ordinary.id],
  );
  assert.equal(selectMarketTimelineMessages([contract, bid], null).length, 2);
});

const relayEvent = (id, content, tags = []) => ({
  id,
  pubkey: PUBKEY,
  kind: 9,
  created_at: 1,
  content,
  tags: [["h", CHANNEL], ...tags],
  sig: "s",
});

function threadPanel(channelEvents, replyEvents, rootId) {
  return buildIndependentThreadPanel(
    channelEvents,
    replyEvents,
    rootId,
    rootId,
    new Set(),
    null,
    PUBKEY,
    null,
    undefined,
    undefined,
    new Map(),
    new Map(),
    null,
    undefined,
  );
}

test("a bid filtered from the main timeline still loads as a thread root", () => {
  const bidId = "6".repeat(64);
  const replyId = "7".repeat(64);
  const bid = relayEvent(bidId, envelope("response"));
  const reply = relayEvent(replyId, "Can you deliver sooner?", [
    ["e", bidId, "", "root"],
    ["e", bidId, "", "reply"],
  ]);
  const panel = threadPanel([bid], [reply], bidId);

  assert.equal(panel.threadHead?.id, bidId);
  assert.deepEqual(
    panel.visibleReplies.map(({ message: entry }) => entry.id),
    [replyId],
  );
});
