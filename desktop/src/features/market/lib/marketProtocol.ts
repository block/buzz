import { z } from "zod";

import type {
  MarketActivity,
  MarketScenario,
  MarketScenarioId,
} from "@/features/market/lib/marketPrototypeData";
import type { UserNote } from "@/shared/api/socialTypes";
import { truncatePubkey } from "@/shared/lib/pubkey";

export const MARKET_PROTOCOL = "buzz-market/v0" as const;

const EventId = z.string().regex(/^[0-9a-f]{64}$/);
const ChannelId = z.string().uuid();
const ActorName = z.string().min(1).max(80);
const PositiveInteger = z.number().int().positive();

const BaseEnvelope = z.object({
  protocol: z.literal(MARKET_PROTOCOL),
  channelId: ChannelId,
});

const AnnouncementEnvelope = BaseEnvelope.extend({
  type: z.literal("announcement"),
  version: z.literal(1),
  listingEventId: EventId,
  listing: z.object({
    actorName: ActorName,
    direction: z.enum(["offer", "request"]),
    mechanism: z.enum(["fixed", "auction", "reverse-auction", "tender"]),
    title: z.string().min(1).max(160),
    summary: z.string().min(1).max(2_000),
    quantity: z.union([PositiveInteger, z.literal("unlimited")]),
    priceSats: PositiveInteger.optional(),
    maxBudgetSats: PositiveInteger.optional(),
    closesAt: PositiveInteger.optional(),
    deliveryMinutes: PositiveInteger.optional(),
    minimumDecrementSats: PositiveInteger.optional(),
    minimumIncrementSats: PositiveInteger.optional(),
    imageUrl: z.string().url().max(500).optional(),
  }),
});

const ContractEnvelope = BaseEnvelope.extend({
  type: z.literal("contract"),
  version: z.literal(1),
  listing: AnnouncementEnvelope.shape.listing,
});

const ResponseEnvelope = BaseEnvelope.extend({
  type: z.literal("response"),
  listingEventId: EventId,
  actorName: ActorName,
  quantity: PositiveInteger,
  amountSats: PositiveInteger.optional(),
  message: z.string().min(1).max(2_000),
});

const AwardEnvelope = BaseEnvelope.extend({
  type: z.literal("award"),
  listingEventId: EventId,
  responseEventId: EventId,
  actorName: ActorName,
  quantity: PositiveInteger,
  amountSats: PositiveInteger,
});

const FulfillmentEnvelope = BaseEnvelope.extend({
  type: z.literal("fulfillment"),
  listingEventId: EventId,
  awardEventId: EventId,
  actorName: ActorName,
  message: z.string().min(1).max(2_000),
});

const SettlementEnvelope = BaseEnvelope.extend({
  type: z.literal("settlement"),
  listingEventId: EventId,
  awardEventId: EventId,
  fulfillmentEventId: EventId,
  actorName: ActorName,
  amountSats: PositiveInteger,
});

export const MarketEnvelopeSchema = z.discriminatedUnion("type", [
  AnnouncementEnvelope,
  ContractEnvelope,
  ResponseEnvelope,
  AwardEnvelope,
  FulfillmentEnvelope,
  SettlementEnvelope,
]);

export type MarketEnvelope = z.infer<typeof MarketEnvelopeSchema>;
export type MarketAnnouncementEnvelope = z.infer<typeof AnnouncementEnvelope>;
export type MarketContractEnvelope = z.infer<typeof ContractEnvelope>;
export type MarketListing = MarketContractEnvelope["listing"];

export type MarketProjection = {
  channelId: string;
  contract: MarketContractEnvelope;
  contractAuthorPubkey: string;
  listingEventId: string;
  bids: MarketBid[];
  scenario: MarketScenario;
  rejected: Array<{ eventId: string; reason: string }>;
  wallet: MarketWallet;
};

export type MarketBid = {
  eventId: string;
  actorName: string;
  amountSats?: number;
  bidderPubkey: string;
  createdAt: number;
  message: string;
  quantity: number;
};

export type MarketWallet = {
  /** Awarded but not yet settled, in fake sats. */
  escrowedSats: number;
  /** Total released by signed settlements, in fake sats. */
  settledSats: number;
};

export function parseMarketEnvelope(content: string): MarketEnvelope | null {
  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch {
    return null;
  }
  const parsed = MarketEnvelopeSchema.safeParse(value);
  return parsed.success ? parsed.data : null;
}

function scenarioIdForListing(
  listing: MarketListing,
  settled: boolean,
): MarketScenarioId {
  if (settled) return "awarded";
  if (
    listing.mechanism === "auction" ||
    listing.mechanism === "reverse-auction"
  ) {
    return "auction";
  }
  if (listing.mechanism === "tender") return "tender";
  return listing.quantity === "unlimited" ? "unlimited" : "finite";
}

function formatAt(timestamp: number): string {
  return new Intl.DateTimeFormat("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "UTC",
    timeZoneName: "short",
  }).format(new Date(timestamp * 1_000));
}

function actorDetail(pubkey: string): string {
  return `agent · ${truncatePubkey(pubkey)}`;
}

function listingPrice(listing: MarketListing): string {
  if (listing.mechanism === "fixed") {
    return `${listing.priceSats ?? 0} sats per unit`;
  }
  if (listing.mechanism === "auction") {
    return `Bidding starts at ${listing.priceSats ?? 0} sats`;
  }
  if (listing.mechanism === "reverse-auction") {
    return `Maximum ${listing.maxBudgetSats ?? 0} sats`;
  }
  return listing.maxBudgetSats
    ? `Budget up to ${listing.maxBudgetSats} sats`
    : "Judged on published criteria";
}

function validateListing(listing: MarketListing): string | null {
  if (listing.mechanism === "fixed" && !listing.priceSats) {
    return "fixed listing requires priceSats";
  }
  if (listing.mechanism === "fixed" && listing.maxBudgetSats) {
    return "fixed listing cannot declare maxBudgetSats";
  }
  if (listing.minimumDecrementSats && listing.mechanism !== "reverse-auction") {
    return "minimumDecrementSats is only valid for reverse auctions";
  }
  if (listing.minimumIncrementSats && listing.mechanism !== "auction") {
    return "minimumIncrementSats is only valid for auctions";
  }
  if (listing.mechanism === "auction" && listing.direction !== "offer") {
    return "auction listing must offer an item";
  }
  if (listing.mechanism === "auction" && listing.quantity !== 1) {
    return "auction listing quantity must be one";
  }
  if (listing.mechanism === "auction" && !listing.priceSats) {
    return "auction requires priceSats";
  }
  if (listing.mechanism === "auction" && !listing.closesAt) {
    return "auction requires closesAt";
  }
  if (listing.mechanism === "auction" && listing.maxBudgetSats) {
    return "auction cannot declare maxBudgetSats";
  }
  if (listing.mechanism === "reverse-auction" && !listing.maxBudgetSats) {
    return "reverse auction requires maxBudgetSats";
  }
  if (listing.mechanism === "reverse-auction" && listing.priceSats) {
    return "reverse auction cannot declare priceSats";
  }
  if (
    listing.mechanism === "reverse-auction" &&
    listing.minimumDecrementSats &&
    listing.minimumDecrementSats > (listing.maxBudgetSats ?? 0)
  ) {
    return "minimum decrement exceeds maximum budget";
  }
  return null;
}

function responseIsValid(
  listing: MarketListing,
  response: Extract<MarketEnvelope, { type: "response" }>,
  bestAuctionBid: number | null,
): string | null {
  if (
    listing.quantity !== "unlimited" &&
    response.quantity > listing.quantity
  ) {
    return "response quantity exceeds listing quantity";
  }
  if (
    listing.mechanism === "fixed" &&
    response.amountSats !== listing.priceSats
  ) {
    return "fixed-price response must match priceSats";
  }
  if (listing.mechanism === "auction") {
    if (!response.amountSats) return "auction response requires amountSats";
    const increment = listing.minimumIncrementSats ?? 1;
    const minimumBid =
      bestAuctionBid === null
        ? (listing.priceSats ?? 0)
        : bestAuctionBid + increment;
    if (response.amountSats < minimumBid) {
      return "bid does not meet minimum increment";
    }
  }
  if (listing.mechanism === "reverse-auction") {
    if (!response.amountSats)
      return "reverse-auction response requires amountSats";
    if (listing.maxBudgetSats && response.amountSats > listing.maxBudgetSats) {
      return "bid exceeds maximum budget";
    }
    const decrement = listing.minimumDecrementSats ?? 1;
    if (
      bestAuctionBid !== null &&
      response.amountSats > bestAuctionBid - decrement
    ) {
      return "bid does not meet minimum decrement";
    }
  }
  return null;
}

function envelopePhase(envelope: MarketEnvelope): number {
  switch (envelope.type) {
    case "announcement":
      return -1;
    case "contract":
      return 0;
    case "response":
      return 1;
    case "award":
      return 2;
    case "fulfillment":
      return 3;
    case "settlement":
      return 4;
  }
}

function hasChannelTag(note: UserNote, channelId: string): boolean {
  return note.tags.some(
    (tag) =>
      tag[0] === "h" && tag[1]?.toLowerCase() === channelId.toLowerCase(),
  );
}

function isTopLevelChannelEvent(note: UserNote): boolean {
  return !note.tags.some((tag) => tag[0] === "e");
}

export function projectMarketChannel(
  events: UserNote[],
  channelId: string,
): MarketProjection | null {
  const parsed = events
    .map((note) => ({ note, envelope: parseMarketEnvelope(note.content) }))
    .filter(
      (entry): entry is { note: UserNote; envelope: MarketEnvelope } =>
        entry.envelope !== null &&
        entry.envelope.channelId.toLowerCase() === channelId.toLowerCase() &&
        hasChannelTag(entry.note, channelId),
    )
    .sort(
      (left, right) =>
        left.note.createdAt - right.note.createdAt ||
        envelopePhase(left.envelope) - envelopePhase(right.envelope) ||
        left.note.id.localeCompare(right.note.id),
    );
  const listingEntry = parsed.find(
    (entry) =>
      entry.envelope.type === "contract" &&
      isTopLevelChannelEvent(entry.note) &&
      validateListing(entry.envelope.listing) === null,
  );
  if (listingEntry?.envelope.type !== "contract") return null;

  const listingEventId = listingEntry.note.id;
  const listing = listingEntry.envelope;
  const marketEvents = parsed.filter(
    (entry) =>
      (entry.envelope.type === "contract" ||
        entry.envelope.type === "response" ||
        entry.envelope.type === "award" ||
        entry.envelope.type === "fulfillment" ||
        entry.envelope.type === "settlement") &&
      entry.note.createdAt >= listingEntry.note.createdAt,
  ) as Array<{
    note: UserNote;
    envelope: Exclude<MarketEnvelope, { type: "announcement" }>;
  }>;
  const responses = new Map<
    string,
    { note: UserNote; envelope: Extract<MarketEnvelope, { type: "response" }> }
  >();
  const awards = new Map<
    string,
    { note: UserNote; envelope: Extract<MarketEnvelope, { type: "award" }> }
  >();
  const fulfillments = new Map<
    string,
    {
      note: UserNote;
      envelope: Extract<MarketEnvelope, { type: "fulfillment" }>;
    }
  >();
  const settlements = new Map<string, UserNote>();
  const acceptedEventIds = new Set<string>();
  const rejected: MarketProjection["rejected"] = [];
  let awardedQuantity = 0;
  let bestAuctionBid: number | null = null;
  let winningAuctionResponseId: string | null = null;

  for (const entry of marketEvents) {
    const { envelope, note } = entry;
    if (envelope.type === "contract") {
      if (note.id !== listingEventId) {
        rejected.push({
          eventId: note.id,
          reason: "channel already has a canonical contract",
        });
      } else {
        acceptedEventIds.add(note.id);
      }
      continue;
    }
    if (envelope.listingEventId !== listingEventId) {
      rejected.push({
        eventId: note.id,
        reason: "listingEventId does not target channel contract",
      });
      continue;
    }
    if (
      envelope.type === "response" &&
      listing.listing.closesAt &&
      note.createdAt > listing.listing.closesAt
    ) {
      rejected.push({
        eventId: note.id,
        reason: "response arrived after listing close",
      });
      continue;
    }

    if (envelope.type === "response") {
      const reason = responseIsValid(listing.listing, envelope, bestAuctionBid);
      if (reason) {
        rejected.push({ eventId: note.id, reason });
        continue;
      }
      responses.set(note.id, { note, envelope });
      acceptedEventIds.add(note.id);
      if (
        listing.listing.mechanism === "auction" ||
        listing.listing.mechanism === "reverse-auction"
      ) {
        bestAuctionBid = envelope.amountSats ?? bestAuctionBid;
        winningAuctionResponseId = note.id;
      }
      continue;
    }

    if (envelope.type === "award") {
      const response = responses.get(envelope.responseEventId);
      const available =
        listing.listing.quantity === "unlimited"
          ? Number.POSITIVE_INFINITY
          : listing.listing.quantity - awardedQuantity;
      const reason =
        note.pubkey !== listingEntry.note.pubkey
          ? "only listing author may award"
          : !response
            ? "award references unknown response"
            : awards.has(envelope.responseEventId)
              ? "response already awarded"
              : listing.listing.mechanism === "auction" &&
                  envelope.responseEventId !== winningAuctionResponseId
                ? "auction award must select highest bid"
                : listing.listing.mechanism === "auction" &&
                    listing.listing.closesAt &&
                    note.createdAt <= listing.listing.closesAt
                  ? "auction cannot be awarded before close"
                  : envelope.quantity > response.envelope.quantity
                    ? "award quantity exceeds response quantity"
                    : envelope.quantity > available
                      ? "award quantity exceeds available quantity"
                      : listing.listing.mechanism !== "tender" &&
                          envelope.amountSats !== response.envelope.amountSats
                        ? "award amount differs from response"
                        : null;
      if (reason) {
        rejected.push({ eventId: note.id, reason });
        continue;
      }
      awards.set(envelope.responseEventId, { note, envelope });
      acceptedEventIds.add(note.id);
      awardedQuantity += envelope.quantity;
      continue;
    }

    if (envelope.type === "fulfillment") {
      const award = [...awards.values()].find(
        (value) => value.note.id === envelope.awardEventId,
      );
      const response = award
        ? responses.get(award.envelope.responseEventId)
        : null;
      const fulfiller =
        listing.listing.direction === "offer"
          ? listingEntry.note.pubkey
          : response?.note.pubkey;
      const reason = !award
        ? "fulfillment references unknown award"
        : note.pubkey !== fulfiller
          ? "fulfillment author is not the delivering agent"
          : fulfillments.has(envelope.awardEventId)
            ? "award already fulfilled"
            : null;
      if (reason) {
        rejected.push({ eventId: note.id, reason });
        continue;
      }
      fulfillments.set(envelope.awardEventId, { note, envelope });
      acceptedEventIds.add(note.id);
      continue;
    }

    const award = [...awards.values()].find(
      (value) => value.note.id === envelope.awardEventId,
    );
    const response = award
      ? responses.get(award.envelope.responseEventId)
      : null;
    const fulfillment = fulfillments.get(envelope.awardEventId);
    const payer =
      listing.listing.direction === "offer"
        ? response?.note.pubkey
        : listingEntry.note.pubkey;
    const reason =
      !award || !fulfillment
        ? "settlement requires a fulfilled award"
        : envelope.fulfillmentEventId !== fulfillment.note.id
          ? "settlement references wrong fulfillment"
          : note.pubkey !== payer
            ? "settlement author is not the payer"
            : envelope.amountSats !==
                award.envelope.amountSats * award.envelope.quantity
              ? "settlement amount differs from award total"
              : settlements.has(envelope.awardEventId)
                ? "award already settled"
                : null;
    if (reason) {
      rejected.push({ eventId: note.id, reason });
      continue;
    }
    settlements.set(envelope.awardEventId, note);
    acceptedEventIds.add(note.id);
  }

  const accepted = marketEvents.filter((entry) => {
    if (!acceptedEventIds.has(entry.note.id)) return false;
    acceptedEventIds.delete(entry.note.id);
    return true;
  });
  const activities: MarketActivity[] = accepted
    .map(({ note, envelope }): MarketActivity => {
      if (envelope.type === "contract") {
        return {
          actor: envelope.listing.actorName,
          at: formatAt(note.createdAt),
          detail: `Contract v${envelope.version} is signed in this Buzz channel and announced on Pulse.`,
          state: "accepted",
          title: "Market opened",
        };
      }
      if (envelope.type === "response") {
        return {
          actor: envelope.actorName,
          at: formatAt(note.createdAt),
          detail: envelope.message,
          state: "discussion",
          title:
            listing.listing.mechanism === "auction" ||
            listing.listing.mechanism === "reverse-auction"
              ? "Bid submitted"
              : "Response submitted",
        };
      }
      if (envelope.type === "award") {
        return {
          actor: envelope.actorName,
          at: formatAt(note.createdAt),
          detail: `Awarded ${envelope.quantity} unit for ${envelope.amountSats} sats each.`,
          state: "terminal",
          title: "Response awarded",
        };
      }
      if (envelope.type === "fulfillment") {
        return {
          actor: envelope.actorName,
          at: formatAt(note.createdAt),
          detail: envelope.message,
          state: "terminal",
          title: "Delivery fulfilled",
        };
      }
      return {
        actor: envelope.actorName,
        at: formatAt(note.createdAt),
        detail: `${envelope.amountSats} fake sats released for the fulfilled award.`,
        state: "terminal",
        title: "Sandbox settlement complete",
      };
    })
    .reverse();

  const settled = settlements.size > 0;
  const bids: MarketBid[] = [...responses.entries()]
    .map(([eventId, { envelope, note }]) => ({
      eventId,
      actorName: envelope.actorName,
      amountSats: envelope.amountSats,
      bidderPubkey: note.pubkey,
      createdAt: note.createdAt,
      message: envelope.message,
      quantity: envelope.quantity,
    }))
    .sort(
      (left, right) =>
        left.createdAt - right.createdAt ||
        left.eventId.localeCompare(right.eventId),
    );
  const fulfilled = fulfillments.size;
  const scenarioId = scenarioIdForListing(listing.listing, settled);
  const quantity = listing.listing.quantity;
  const available =
    quantity === "unlimited"
      ? "Unlimited"
      : `${quantity - awardedQuantity} of ${quantity}`;
  const directionLabel =
    listing.listing.direction === "offer" ? "Seller" : "Requester";
  const closeAt = listing.listing.closesAt
    ? `Closes ${new Date(listing.listing.closesAt * 1_000).toISOString().replace("T", " ").slice(0, 16)} UTC`
    : quantity === "unlimited"
      ? "No quantity limit"
      : "No scheduled close · UTC";
  const status: MarketScenario["status"] = settled
    ? "Fulfilled"
    : awards.size > 0
      ? "Awarded"
      : listing.listing.closesAt &&
          listing.listing.closesAt < Date.now() / 1_000
        ? "Closed"
        : "Open";
  const auctionNextBid =
    bestAuctionBid === null
      ? (listing.listing.priceSats ?? 0)
      : bestAuctionBid + (listing.listing.minimumIncrementSats ?? 1);
  let escrowedSats = 0;
  let settledSats = 0;
  for (const award of awards.values()) {
    const total = award.envelope.amountSats * award.envelope.quantity;
    if (settlements.has(award.note.id)) settledSats += total;
    else escrowedSats += total;
  }

  return {
    channelId,
    contract: listing,
    contractAuthorPubkey: listingEntry.note.pubkey,
    listingEventId,
    bids,
    rejected,
    wallet: { escrowedSats, settledSats },
    scenario: {
      id: scenarioId,
      eyebrow: `${listing.listing.direction === "offer" ? "Offer" : "Request"} · ${listing.listing.mechanism.replace("-", " ")} · ${quantity === "unlimited" ? "unlimited" : "finite"}`,
      title: listing.listing.title,
      summary: listing.listing.summary,
      imageUrl: listing.listing.imageUrl,
      direction:
        listing.listing.direction === "offer"
          ? `Buyer pays ${listingPrice(listing.listing)} · Seller delivers`
          : `Requester pays ${listingPrice(listing.listing)} · Winning agent delivers`,
      mode:
        listing.listing.mechanism === "fixed"
          ? "Fixed price"
          : listing.listing.mechanism === "auction"
            ? "Highest-bid auction"
            : listing.listing.mechanism === "reverse-auction"
              ? "Reverse auction"
              : "Qualitative tender",
      status,
      statusDetail: `Channel state ${accepted.at(-1)?.note.id.slice(0, 8) ?? listingEventId.slice(0, 8)} · ${formatAt(accepted.at(-1)?.note.createdAt ?? listingEntry.note.createdAt)}`,
      closeAt,
      contractId: `${MARKET_PROTOCOL}:${channelId}:v${listing.version}`,
      primaryAction:
        listing.listing.mechanism === "auction"
          ? `Bid at least ${auctionNextBid} sats`
          : listing.listing.mechanism === "reverse-auction"
            ? `Bid below ${bestAuctionBid ?? listing.listing.maxBudgetSats ?? 0} sats`
            : `Respond for ${listing.listing.priceSats ?? listing.listing.maxBudgetSats ?? 0} sats`,
      terms: [
        {
          label: directionLabel,
          value: listing.listing.actorName,
          detail: actorDetail(listingEntry.note.pubkey),
        },
        {
          label: listing.listing.direction === "offer" ? "Price" : "Reward",
          value: listingPrice(listing.listing),
          detail: "fake-sats sandbox",
        },
        { label: "Initial quantity", value: String(quantity) },
        ...(listing.listing.deliveryMinutes
          ? [
              {
                label: "Delivery deadline",
                value: `${listing.listing.deliveryMinutes} minutes after award`,
              },
            ]
          : []),
        {
          label: "Contract version",
          value: `v${listing.version} · event ${listingEventId.slice(0, 8)}`,
        },
        { label: "Settlement", value: "Signed fake-sats receipt" },
      ],
      liveMetrics: [
        { label: "Available", value: available },
        ...(listing.listing.mechanism === "auction"
          ? [
              {
                label: "Highest bid",
                value:
                  bestAuctionBid === null
                    ? "No bids"
                    : `${bestAuctionBid} sats`,
              },
            ]
          : []),
        { label: "Responses", value: String(responses.size) },
        { label: "Awarded", value: String(awards.size) },
        {
          label: "Fulfilled / settled",
          value: `${fulfilled} / ${settlements.size}`,
        },
      ],
      activity: activities,
    },
  };
}

export function projectMarketAnnouncements(
  notes: UserNote[],
): MarketAnnouncement[] {
  const announcements = notes.flatMap((note) => {
    const envelope = parseMarketEnvelope(note.content);
    if (envelope?.type !== "announcement") return [];
    const listingReason = validateListing(envelope.listing);
    if (listingReason) return [];
    return [{ note, envelope }];
  });
  const seenChannels = new Set<string>();
  return announcements
    .sort(
      (left, right) =>
        left.note.createdAt - right.note.createdAt ||
        left.note.id.localeCompare(right.note.id),
    )
    .flatMap(({ note, envelope }) => {
      const channelKey = envelope.channelId.toLowerCase();
      if (seenChannels.has(channelKey)) return [];
      seenChannels.add(channelKey);
      return [
        {
          announcementEventId: note.id,
          channelId: envelope.channelId,
          createdAt: note.createdAt,
          listingEventId: envelope.listingEventId,
          listing: envelope.listing,
          publisherPubkey: note.pubkey,
        },
      ];
    })
    .sort(
      (left, right) =>
        right.createdAt - left.createdAt ||
        right.announcementEventId.localeCompare(left.announcementEventId),
    );
}

export function marketAnnouncementMatchesProjection(
  announcement: MarketAnnouncement,
  projection: MarketProjection,
): boolean {
  return (
    announcement.channelId.toLowerCase() ===
      projection.channelId.toLowerCase() &&
    announcement.listingEventId === projection.listingEventId &&
    announcement.publisherPubkey.toLowerCase() ===
      projection.contractAuthorPubkey.toLowerCase() &&
    listingsMatch(announcement.listing, projection.contract.listing)
  );
}

function listingsMatch(left: MarketListing, right: MarketListing): boolean {
  return AnnouncementEnvelope.shape.listing
    .keyof()
    .options.every((key) => left[key] === right[key]);
}

export type MarketAnnouncement = {
  announcementEventId: string;
  channelId: string;
  createdAt: number;
  listingEventId: string;
  listing: MarketAnnouncementEnvelope["listing"];
  publisherPubkey: string;
};
