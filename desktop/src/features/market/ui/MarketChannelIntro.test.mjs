import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { MARKET_SCENARIOS } from "@/features/market/lib/marketPrototypeData";
import { MarketBidTable } from "./MarketChannelIntro.tsx";
import { MarketContractCard } from "./MarketContractCard.tsx";

const bidderOne = "a".repeat(64);
const bidderTwo = "b".repeat(64);
const bids = [
  {
    eventId: "1".repeat(64),
    actorName: "Signed name one",
    bidderPubkey: bidderOne,
    createdAt: 1,
    amountSats: 40,
    quantity: 1,
    message: "First bid",
  },
  {
    eventId: "2".repeat(64),
    actorName: "Signed name two",
    bidderPubkey: bidderTwo,
    createdAt: 2,
    amountSats: 35,
    quantity: 2,
    message: "Second bid",
  },
];
const profiles = {
  [bidderOne]: { displayName: "Fizz", avatarUrl: null },
  [bidderTwo]: { displayName: "Honey", avatarUrl: null },
};

test("the bid table renders exactly one body row per bid", () => {
  const html = renderToStaticMarkup(
    React.createElement(MarketBidTable, {
      bids,
      onOpenBid: () => {},
      profiles,
    }),
  );
  const body = html.match(/<tbody[\s\S]*?<\/tbody>/)?.[0] ?? "";

  assert.equal(body.match(/<tr/g)?.length ?? 0, bids.length);
  assert.match(html, />Bidder</);
  assert.match(html, />Bid</);
  assert.match(html, />Message</);
  assert.match(html, />Fizz</);
  assert.match(html, />Honey</);
  assert.match(html, />First bid</);
  assert.match(html, />Second bid</);
});

test("the contract card has no independent horizontal margin", () => {
  const html = renderToStaticMarkup(
    React.createElement(MarketContractCard, {
      scenario: MARKET_SCENARIOS.finite,
    }),
  );
  const sectionClass = html.match(/<section class="([^"]+)"/)?.[1] ?? "";

  assert.doesNotMatch(sectionClass, /(?:^|\s)m[lrxy]-/);
});
