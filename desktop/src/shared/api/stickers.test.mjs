import assert from "node:assert/strict";
import test from "node:test";

import {
  catalogEntriesFromEvent,
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
