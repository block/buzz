import assert from "node:assert/strict";
import test from "node:test";

import {
  getTranscriptHistoryCertainty,
  getUncertainHistoryCopy,
  isUncertainHistory,
} from "./agentSessionHistoryCertainty.ts";

const complete = {
  hasSubscription: true,
  channelId: "channel-1",
  archiveHydrated: true,
};

test("a fully checked channel can state emptiness as fact", () => {
  assert.equal(getTranscriptHistoryCertainty(complete), "known-empty");
  assert.equal(isUncertainHistory("known-empty"), false);
});

test("an unresolved subscription check is never a definitive empty", () => {
  const certainty = getTranscriptHistoryCertainty({
    ...complete,
    hasSubscription: null,
  });
  assert.equal(certainty, "unknown-subscription-unresolved");
  assert.equal(isUncertainHistory(certainty), true);
});

test("a missing owner_p subscription is never a definitive empty", () => {
  assert.equal(
    getTranscriptHistoryCertainty({ ...complete, hasSubscription: false }),
    "unknown-not-indexed",
  );
});

test("an unhydrated archive is never a definitive empty", () => {
  assert.equal(
    getTranscriptHistoryCertainty({ ...complete, archiveHydrated: false }),
    "unknown-archive-loading",
  );
});

test("no channel means no archive was consulted", () => {
  assert.equal(
    getTranscriptHistoryCertainty({ ...complete, channelId: null }),
    "unknown-archive-loading",
  );
});

test("a missing subscription outranks a missing channel", () => {
  // Order matters for the copy: without a subscription the archive was never
  // going to load, so "isn't indexed" is the honest reason rather than
  // "still loading", which implies waiting would help.
  assert.equal(
    getTranscriptHistoryCertainty({
      hasSubscription: false,
      channelId: null,
      archiveHydrated: false,
    }),
    "unknown-not-indexed",
  );
});

const uncertainVariants = [
  "unknown-subscription-unresolved",
  "unknown-not-indexed",
  "unknown-archive-loading",
];

test("uncertain copy never claims there was no activity", () => {
  for (const certainty of uncertainVariants) {
    const copy = getUncertainHistoryCopy(certainty);
    const text = `${copy.title} ${copy.description}`.toLowerCase();
    assert.ok(!text.includes("no acp activity"), text);
    assert.ok(!text.includes("no activity"), text);
    assert.ok(copy.description.length > 0);
  }
});

test("uncertain copy names the specific gap", () => {
  assert.match(
    getUncertainHistoryCopy("unknown-subscription-unresolved").title,
    /Checking/,
  );
  assert.match(
    getUncertainHistoryCopy("unknown-not-indexed").description,
    /isn't indexed/,
  );
  assert.match(
    getUncertainHistoryCopy("unknown-archive-loading").description,
    /hasn't finished loading/,
  );
});

test("every uncertain variant has distinct copy", () => {
  const titles = uncertainVariants.map(
    (certainty) => getUncertainHistoryCopy(certainty).title,
  );
  assert.equal(new Set(titles).size, uncertainVariants.length);
});
