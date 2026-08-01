import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  catalogEntriesFromEvent,
  fetchAllStickerPacks,
  fetchStickerCatalog,
  parseStickerPack,
  parseStickerReference,
  stickerAssetCacheUrl,
} from "./stickers.ts";

const author = "a".repeat(64);
const hash = "b".repeat(64);
const eventId = "c".repeat(64);
const coordinate = `30031:${author}:hello`;

function packEvent(overrides = {}) {
  return {
    id: eventId,
    pubkey: author,
    created_at: 1,
    kind: 30031,
    content: "",
    sig: "d".repeat(128),
    tags: [
      ["d", "hello"],
      ["title", "Hello"],
      ["pack_format", "sonar-sticker-pack-v1"],
      ["t", "sonar-sticker-pack-v1"],
      [
        "sticker",
        "wave",
        `https://relay.example/media/${hash}.webp`,
        hash,
        "image/webp",
        "512x512",
        "Wave",
        "👋",
      ],
      ["emoji", "wave", `https://relay.example/media/${hash}.webp`],
    ],
    ...overrides,
  };
}

test("parses a canonical Sonar sticker pack", () => {
  const pack = parseStickerPack(packEvent());
  assert.equal(pack?.coordinate, coordinate);
  assert.equal(pack?.stickers[0]?.shortcode, "wave");
  assert.equal(
    stickerAssetCacheUrl(pack, pack.stickers[0]),
    `/media/sticker/${author}/hello/wave/${hash}`,
  );
});

test("rejects non-empty pack content and noncanonical dimensions", () => {
  assert.equal(parseStickerPack(packEvent({ content: "secret" })), null);
  const malformed = packEvent();
  malformed.tags[4][5] = "8192x8192";
  assert.equal(parseStickerPack(malformed), null);
});

test("compatibility emoji tags are optional but must be unique and exact", () => {
  const minimal = packEvent();
  minimal.tags = minimal.tags
    .filter((tag) => tag[0] !== "t" && tag[0] !== "emoji")
    .map((tag) => (tag[0] === "sticker" ? tag.slice(0, 6) : tag));
  assert.equal(parseStickerPack(minimal)?.stickers[0]?.shortcode, "wave");

  const duplicate = packEvent();
  duplicate.tags.push([
    "emoji",
    "wave",
    `https://relay.example/media/${hash}.webp`,
  ]);
  assert.equal(parseStickerPack(duplicate), null);
});

test("message sticker references require exact lowercase four-field tags", () => {
  assert.deepEqual(
    parseStickerReference([["sticker", coordinate, "wave", hash]]),
    {
      coordinate,
      author,
      identifier: "hello",
      shortcode: "wave",
      sha256: hash,
    },
  );
  assert.equal(
    parseStickerReference([
      ["sticker", coordinate, "wave", hash.toUpperCase()],
    ]),
    null,
  );
  assert.equal(
    parseStickerReference([["sticker", coordinate, "wave", hash, "extra"]]),
    null,
  );
});

test("catalog entries pin the approved event id and reject uppercase ids", () => {
  const catalog = {
    ...packEvent(),
    kind: 13536,
    tags: [["-"], ["a", coordinate, eventId]],
  };
  assert.deepEqual(catalogEntriesFromEvent(catalog), [
    { coordinate, approvedEventId: eventId },
  ]);
  catalog.tags[1][2] = eventId.toUpperCase();
  assert.deepEqual(catalogEntriesFromEvent(catalog), []);
});

test("catalog and message coordinates must remain canonical and exact", () => {
  const uppercaseCoordinate = `30031:${author.toUpperCase()}:hello`;
  assert.equal(
    parseStickerReference([["sticker", uppercaseCoordinate, "wave", hash]]),
    null,
  );
  const catalog = {
    ...packEvent(),
    kind: 13536,
    tags: [["-"], ["a", coordinate, eventId, "extra"]],
  };
  assert.deepEqual(catalogEntriesFromEvent(catalog), []);

  catalog.tags = [["-"], ["client", "buzz"], ["a", coordinate, eventId]];
  assert.deepEqual(catalogEntriesFromEvent(catalog), []);
});

// `buzz-core::stickers::validate_asset_tag` lowercases the URL path before
// looking for the plaintext hash, so the relay accepts an asset URL that
// renders the hash in uppercase hex. Desktop parsing must agree, or an approved
// pack silently degrades to a "superseded" placeholder no member can install.
test("asset URLs may render the sha256 in uppercase hex, matching the relay", () => {
  const uppercaseUrl = `https://relay.example/media/${hash.toUpperCase()}.webp`;
  const event = packEvent();
  event.tags[4][2] = uppercaseUrl;
  event.tags[5][2] = uppercaseUrl;
  const pack = parseStickerPack(event);
  assert.equal(pack?.stickers[0]?.url, uppercaseUrl);
  assert.equal(pack?.stickers[0]?.sha256, hash);

  // The hash itself must still be canonical lowercase in its own field.
  const uppercaseHashField = packEvent();
  uppercaseHashField.tags[4][3] = hash.toUpperCase();
  assert.equal(parseStickerPack(uppercaseHashField), null);
});

// A superseded approval must never look like an ordinary installable pack:
// `useInstalledStickerPacks` and `ComposerStickerPicker` both gate on this flag
// so members cannot install a coordinate that resolves to zero stickers.
test("a superseded catalog approval is flagged, not returned as installable", async (t) => {
  t.after(() => mock.restoreAll());
  const catalog = {
    ...packEvent(),
    kind: 13536,
    tags: [["-"], ["a", coordinate, eventId]],
  };
  mock.method(relayClient, "fetchEvents", (filter) =>
    Promise.resolve(filter.kinds[0] === 13536 ? [catalog] : []),
  );

  const packs = await fetchStickerCatalog();

  assert.equal(packs.length, 1);
  assert.equal(packs[0].coordinate, coordinate);
  assert.equal(packs[0].superseded, true);
  assert.deepEqual(packs[0].stickers, []);
});

function packPage(count, startIndex, createdAt) {
  return Array.from({ length: count }, (_, index) => {
    const identifier = `pack-${startIndex + index}`;
    const event = packEvent({
      id: (startIndex + index).toString(16).padStart(64, "0"),
      created_at:
        typeof createdAt === "function" ? createdAt(index) : createdAt,
    });
    event.tags[0] = ["d", identifier];
    event.tags[1] = ["title", identifier];
    return event;
  });
}

function stubPagedRelay(pages) {
  const filters = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    filters.push(filter);
    return Promise.resolve(pages[filters.length - 1] ?? []);
  });
  return filters;
}

// The admin approval queue is derived entirely from this list, so a single
// limit-capped fetch makes every pack past the newest page impossible to
// approve — even while the catalog still has capacity.
test("published packs past the first page stay discoverable", async (t) => {
  t.after(() => mock.restoreAll());
  const filters = stubPagedRelay([
    packPage(500, 0, (index) => 10_000 - index),
    packPage(3, 500, 9_000),
  ]);

  const packs = await fetchAllStickerPacks();

  assert.equal(filters.length, 2, "a full page must be followed by another");
  assert.equal(filters[0].until, undefined, "the first page has no cursor");
  assert.equal(
    filters[1].until,
    10_000 - 499,
    "the cursor must be the oldest created_at from the previous page",
  );
  assert.equal(packs.length, 503);
});

test("a short first page of published packs issues no second request", async (t) => {
  t.after(() => mock.restoreAll());
  const filters = stubPagedRelay([packPage(2, 0, 10_000)]);

  const packs = await fetchAllStickerPacks();

  assert.equal(filters.length, 1);
  assert.equal(packs.length, 2);
});

// `until` is inclusive on the relay, so consecutive pages overlap on the
// boundary timestamp. Without id dedupe the repeats would be counted twice.
test("overlapping published-pack pages are deduped by event id", async (t) => {
  t.after(() => mock.restoreAll());
  const firstPage = packPage(500, 0, (index) => 10_000 - index);
  const secondPage = [
    firstPage[firstPage.length - 1],
    ...packPage(2, 500, 9_000),
  ];
  stubPagedRelay([firstPage, secondPage]);

  const packs = await fetchAllStickerPacks();

  assert.equal(packs.length, 502, "the repeated event must count once");
});

// Stop-on-no-progress: a full page whose events all share one created_at cannot
// advance the cursor, so paging must terminate instead of looping forever.
test("a full published-pack page of tied timestamps terminates the walk", async (t) => {
  t.after(() => mock.restoreAll());
  const tiedPage = packPage(500, 0, 10_000);
  const filters = stubPagedRelay([tiedPage, tiedPage, tiedPage, tiedPage]);

  const packs = await fetchAllStickerPacks();

  assert.equal(
    filters.length,
    2,
    "the walk must stop once a page contributes nothing new",
  );
  assert.equal(packs.length, 500);
});
