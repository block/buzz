import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { MARKET_SCENARIOS } from "@/features/market/lib/marketPrototypeData";
import { MarketBidTable, MarketBoardLayout } from "./MarketChannelIntro.tsx";
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
  assert.doesNotMatch(html, />Message</);
  assert.match(html, />Fizz</);
  assert.match(html, />Honey</);
  assert.doesNotMatch(html, />First bid</);
  assert.doesNotMatch(html, />Second bid</);
});

test("the market board follows the header and avatar gutter", () => {
  const html = renderToStaticMarkup(
    React.createElement(MarketBoardLayout, null, "Board"),
  );
  const className = html.match(/<div class="([^"]+)"/)?.[1] ?? "";

  assert.match(className, /(?:^|\s)mx-3(?:\s|$)/);
});

test("the contract card leaves horizontal alignment to the board", () => {
  const html = renderToStaticMarkup(
    React.createElement(MarketContractCard, {
      scenario: MARKET_SCENARIOS.finite,
    }),
  );
  const sectionClass = html.match(/<section class="([^"]+)"/)?.[1] ?? "";

  assert.doesNotMatch(sectionClass, /(?:^|\s)m[lrxy]-/);
});
