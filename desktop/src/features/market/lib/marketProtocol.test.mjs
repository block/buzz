import assert from "node:assert/strict";
import test from "node:test";

import {
  MARKET_PROTOCOL,
  marketAnnouncementMatchesProjection,
  parseMarketEnvelope,
  projectMarketAnnouncements,
  projectMarketChannel,
} from "./marketProtocol.ts";

const CHANNEL_ID = "123e4567-e89b-12d3-a456-426614174000";
const OTHER_CHANNEL_ID = "123e4567-e89b-12d3-a456-426614174001";
const SELLER = "a".repeat(64);
const BUYER = "b".repeat(64);
const LISTING_ID = "1".repeat(64);
const RESPONSE_ID = "2".repeat(64);
const AWARD_ID = "3".repeat(64);
const FULFILLMENT_ID = "4".repeat(64);

function note(
  id,
  pubkey,
  createdAt,
  envelope,
  tags = [["h", envelope.channelId]],
) {
  return { id, pubkey, createdAt, content: JSON.stringify(envelope), tags };
}

function listing(overrides = {}) {
  return {
    actorName: "Seller Agent",
    direction: "offer",
    mechanism: "fixed",
    title: "Incident report",
    summary: "A cited report delivered after award.",
    quantity: 1,
    priceSats: 50,
    deliveryMinutes: 120,
    ...overrides,
  };
}

function contract(overrides = {}) {
  return {
    protocol: MARKET_PROTOCOL,
    type: "contract",
    channelId: CHANNEL_ID,
    version: 1,
    listing: listing(overrides),
  };
}

function announcement(overrides = {}) {
  return {
    ...contract(overrides),
    type: "announcement",
    listingEventId: LISTING_ID,
  };
}

function lifecycle(type, fields) {
  return {
    protocol: MARKET_PROTOCOL,
    type,
    channelId: CHANNEL_ID,
    listingEventId: LISTING_ID,
    ...fields,
  };
}

test("parseMarketEnvelope accepts channel contracts and rejects legacy feeds", () => {
  assert.equal(parseMarketEnvelope("hello Pulse"), null);
  assert.equal(parseMarketEnvelope('{"protocol":"other"}'), null);
  assert.equal(
    parseMarketEnvelope(JSON.stringify(contract()))?.type,
    "contract",
  );
  assert.equal(
    parseMarketEnvelope(
      JSON.stringify({
        ...contract(),
        channelId: undefined,
        marketId: "legacy",
      }),
    ),
    null,
  );
});

test("projectMarketChannel folds a channel contract through fake settlement", () => {
  const events = [
    note(LISTING_ID, SELLER, 100, contract()),
    note(
      RESPONSE_ID,
      BUYER,
      101,
      lifecycle("response", {
        actorName: "Buyer Agent",
        quantity: 1,
        amountSats: 50,
        message: "I reserve the report.",
      }),
    ),
    note(
      AWARD_ID,
      SELLER,
      102,
      lifecycle("award", {
        responseEventId: RESPONSE_ID,
        actorName: "Seller Agent",
        quantity: 1,
        amountSats: 50,
      }),
    ),
    note(
      FULFILLMENT_ID,
      SELLER,
      103,
      lifecycle("fulfillment", {
        awardEventId: AWARD_ID,
        actorName: "Seller Agent",
        message: "Report delivered with cited evidence.",
      }),
    ),
    note(
      "5".repeat(64),
      BUYER,
      104,
      lifecycle("settlement", {
        awardEventId: AWARD_ID,
        fulfillmentEventId: FULFILLMENT_ID,
        actorName: "Buyer Agent",
        amountSats: 50,
      }),
    ),
  ];

  const projection = projectMarketChannel(events, CHANNEL_ID);
  assert.ok(projection);
  assert.equal(projection.channelId, CHANNEL_ID);
  assert.equal(projection.contractAuthorPubkey, SELLER);
  assert.deepEqual(projection.bids, [
    {
      eventId: RESPONSE_ID,
      actorName: "Buyer Agent",
      amountSats: 50,
      bidderPubkey: BUYER,
      createdAt: 101,
      message: "I reserve the report.",
      quantity: 1,
    },
  ]);
  assert.equal(projection.scenario.status, "Fulfilled");
  assert.equal(projection.scenario.activity.length, 5);
  assert.equal(
    projection.scenario.activity[0].title,
    "Sandbox settlement complete",
  );
  assert.deepEqual(projection.rejected, []);
  assert.deepEqual(projection.wallet, { escrowedSats: 0, settledSats: 50 });
});

test("channel tags and the first top-level contract define the canonical market", () => {
  const secondContractId = "8".repeat(64);
  const projection = projectMarketChannel(
    [
      note("9".repeat(64), SELLER, 98, contract(), [["h", OTHER_CHANNEL_ID]]),
      note("7".repeat(64), SELLER, 99, contract(), [
        ["h", CHANNEL_ID],
        ["e", "6".repeat(64), "", "reply"],
      ]),
      note(LISTING_ID, SELLER, 100, contract()),
      note(secondContractId, SELLER, 101, contract({ title: "Replacement" })),
    ],
    CHANNEL_ID,
  );

  assert.ok(projection);
  assert.equal(projection.scenario.title, "Incident report");
  assert.deepEqual(projection.rejected, [
    {
      eventId: secondContractId,
      reason: "channel already has a canonical contract",
    },
  ]);
});

test("projectMarketChannel rejects cross-contract and unauthorized transitions", () => {
  const wrongListingId = "9".repeat(64);
  const projection = projectMarketChannel(
    [
      note(LISTING_ID, SELLER, 100, contract()),
      note(
        RESPONSE_ID,
        BUYER,
        101,
        lifecycle("response", {
          listingEventId: wrongListingId,
          actorName: "Buyer Agent",
          quantity: 1,
          amountSats: 50,
          message: "Wrong contract.",
        }),
      ),
      note(
        "6".repeat(64),
        BUYER,
        102,
        lifecycle("award", {
          responseEventId: RESPONSE_ID,
          actorName: "Buyer Agent",
          quantity: 1,
          amountSats: 50,
        }),
      ),
    ],
    CHANNEL_ID,
  );

  assert.ok(projection);
  assert.equal(projection.scenario.status, "Open");
  assert.deepEqual(
    projection.rejected.map(({ reason }) => reason),
    [
      "listingEventId does not target channel contract",
      "only listing author may award",
    ],
  );
});

test("ascending auction accepts only minimum raises and awards the high bid after close", () => {
  const auction = contract({
    mechanism: "auction",
    priceSats: 50,
    minimumIncrementSats: 10,
    closesAt: 200,
  });
  const secondBidId = "6".repeat(64);
  const projection = projectMarketChannel(
    [
      note(LISTING_ID, SELLER, 100, auction),
      note(
        RESPONSE_ID,
        BUYER,
        101,
        lifecycle("response", {
          actorName: "Bidder One",
          quantity: 1,
          amountSats: 50,
          message: "Opening bid.",
        }),
      ),
      note(
        "7".repeat(64),
        "c".repeat(64),
        102,
        lifecycle("response", {
          actorName: "Bidder Two",
          quantity: 1,
          amountSats: 55,
          message: "Invalid small raise.",
        }),
      ),
      note(
        secondBidId,
        "c".repeat(64),
        103,
        lifecycle("response", {
          actorName: "Bidder Two",
          quantity: 1,
          amountSats: 60,
          message: "Minimum valid raise.",
        }),
      ),
      note(
        "9".repeat(64),
        SELLER,
        104,
        lifecycle("award", {
          responseEventId: secondBidId,
          actorName: "Seller Agent",
          quantity: 1,
          amountSats: 60,
        }),
      ),
      note(
        "8".repeat(64),
        SELLER,
        201,
        lifecycle("award", {
          responseEventId: RESPONSE_ID,
          actorName: "Seller Agent",
          quantity: 1,
          amountSats: 50,
        }),
      ),
      note(
        AWARD_ID,
        SELLER,
        202,
        lifecycle("award", {
          responseEventId: secondBidId,
          actorName: "Seller Agent",
          quantity: 1,
          amountSats: 60,
        }),
      ),
    ],
    CHANNEL_ID,
  );

  assert.ok(projection);
  assert.equal(projection.scenario.mode, "Highest-bid auction");
  assert.deepEqual(projection.scenario.liveMetrics[1], {
    label: "Highest bid",
    value: "60 sats",
  });
  assert.equal(projection.scenario.status, "Awarded");
  assert.deepEqual(
    projection.bids.map(({ amountSats }) => amountSats),
    [50, 60],
  );
  assert.deepEqual(
    projection.rejected.map(({ reason }) => reason),
    [
      "bid does not meet minimum increment",
      "auction cannot be awarded before close",
      "auction award must select highest bid",
    ],
  );
});

test("reverse auction enforces the minimum decrement", () => {
  const auction = contract({
    direction: "request",
    mechanism: "reverse-auction",
    priceSats: undefined,
    maxBudgetSats: 100,
    minimumDecrementSats: 10,
  });
  const projection = projectMarketChannel(
    [
      note(LISTING_ID, SELLER, 100, auction),
      note(
        RESPONSE_ID,
        BUYER,
        101,
        lifecycle("response", {
          actorName: "Bidder One",
          quantity: 1,
          amountSats: 90,
          message: "I bid 90.",
        }),
      ),
      note(
        "6".repeat(64),
        "c".repeat(64),
        102,
        lifecycle("response", {
          actorName: "Bidder Two",
          quantity: 1,
          amountSats: 85,
          message: "I bid 85.",
        }),
      ),
    ],
    CHANNEL_ID,
  );

  assert.ok(projection);
  assert.deepEqual(
    projection.bids.map(({ eventId }) => eventId),
    [RESPONSE_ID],
  );
  assert.equal(projection.scenario.liveMetrics[1].value, "1");
  assert.equal(
    projection.rejected[0].reason,
    "bid does not meet minimum decrement",
  );
});

test("an awarded but unsettled market escrows the award total", () => {
  const projection = projectMarketChannel(
    [
      note(LISTING_ID, SELLER, 100, contract({ quantity: 2 })),
      note(
        RESPONSE_ID,
        BUYER,
        101,
        lifecycle("response", {
          actorName: "Buyer Agent",
          quantity: 2,
          amountSats: 50,
          message: "I reserve two reports.",
        }),
      ),
      note(
        AWARD_ID,
        SELLER,
        102,
        lifecycle("award", {
          responseEventId: RESPONSE_ID,
          actorName: "Seller Agent",
          quantity: 2,
          amountSats: 50,
        }),
      ),
    ],
    CHANNEL_ID,
  );

  assert.ok(projection);
  assert.deepEqual(projection.wallet, { escrowedSats: 100, settledSats: 0 });
});

test("a lifecycle envelope cannot be accepted twice under one event id", () => {
  const sharedEventId = RESPONSE_ID;
  const projection = projectMarketChannel(
    [
      note(LISTING_ID, SELLER, 100, contract()),
      note(
        sharedEventId,
        BUYER,
        101,
        lifecycle("response", {
          actorName: "Buyer Agent",
          quantity: 1,
          amountSats: 50,
          message: "Valid response.",
        }),
      ),
      note(
        sharedEventId,
        BUYER,
        102,
        lifecycle("response", {
          listingEventId: "9".repeat(64),
          actorName: "Buyer Agent",
          quantity: 1,
          amountSats: 50,
          message: "Invalid duplicate fixture.",
        }),
      ),
    ],
    CHANNEL_ID,
  );

  assert.ok(projection);
  assert.equal(
    projection.scenario.activity.filter(
      (activity) => activity.title === "Response submitted",
    ).length,
    1,
  );
  assert.equal(projection.rejected.length, 1);
});

test("responses close on time while awarded work can finish afterward", () => {
  const closedContract = contract({ closesAt: 101 });
  const lateResponseId = "6".repeat(64);
  const projection = projectMarketChannel(
    [
      note(LISTING_ID, SELLER, 100, closedContract),
      note(
        RESPONSE_ID,
        BUYER,
        101,
        lifecycle("response", {
          actorName: "Buyer Agent",
          quantity: 1,
          amountSats: 50,
          message: "On-time response.",
        }),
      ),
      note(
        AWARD_ID,
        SELLER,
        102,
        lifecycle("award", {
          responseEventId: RESPONSE_ID,
          actorName: "Seller Agent",
          quantity: 1,
          amountSats: 50,
        }),
      ),
      note(
        FULFILLMENT_ID,
        SELLER,
        103,
        lifecycle("fulfillment", {
          awardEventId: AWARD_ID,
          actorName: "Seller Agent",
          message: "Delivered after bidding closed.",
        }),
      ),
      note(
        "5".repeat(64),
        BUYER,
        104,
        lifecycle("settlement", {
          awardEventId: AWARD_ID,
          fulfillmentEventId: FULFILLMENT_ID,
          actorName: "Buyer Agent",
          amountSats: 50,
        }),
      ),
      note(
        lateResponseId,
        "c".repeat(64),
        105,
        lifecycle("response", {
          actorName: "Late Buyer",
          quantity: 1,
          amountSats: 50,
          message: "Too late.",
        }),
      ),
    ],
    CHANNEL_ID,
  );

  assert.ok(projection);
  assert.equal(projection.scenario.status, "Fulfilled");
  assert.deepEqual(projection.rejected, [
    {
      eventId: lateResponseId,
      reason: "response arrived after listing close",
    },
  ]);
});

test("Pulse announcements are only index pointers and must match channel truth", () => {
  const pulseNote = note("f".repeat(64), SELLER, 110, announcement(), []);
  const spoofedAnnouncement = note(
    "e".repeat(64),
    BUYER,
    111,
    announcement({ title: "Spoof" }),
    [],
  );
  const announcements = projectMarketAnnouncements([
    pulseNote,
    spoofedAnnouncement,
  ]);
  const projection = projectMarketChannel(
    [note(LISTING_ID, SELLER, 100, contract())],
    CHANNEL_ID,
  );

  assert.equal(announcements.length, 1);
  assert.ok(projection);
  assert.equal(announcements[0].channelId, CHANNEL_ID);
  assert.equal(
    marketAnnouncementMatchesProjection(announcements[0], projection),
    true,
  );
  assert.equal(
    marketAnnouncementMatchesProjection(
      { ...announcements[0], publisherPubkey: BUYER },
      projection,
    ),
    false,
  );
  assert.equal(
    marketAnnouncementMatchesProjection(
      {
        ...announcements[0],
        listing: { ...announcements[0].listing, title: "Spoof" },
      },
      projection,
    ),
    false,
  );
});
