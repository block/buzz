import assert from "node:assert/strict";
import test from "node:test";

import {
  filterGifFavorites,
  isGifFavorite,
  isStoredGifFavorite,
  MAX_GIF_FAVORITES,
  parseGifFavorites,
  toggleGifFavorite,
} from "./favorites.ts";

const MEDIA = "https://media1.giphy.com/media";

function gif(id, overrides = {}) {
  return {
    id,
    original: {
      height: 360,
      size: 42,
      url: `${MEDIA}/${id}/giphy-downsized.gif`,
      width: 640,
    },
    poster: {
      height: 112,
      url: `${MEDIA}/${id}/200w_s.gif`,
      width: 200,
    },
    preview: {
      height: 112,
      url: `${MEDIA}/${id}/200w.webp`,
      width: 200,
    },
    slug: `party-${id}`,
    title: `Party ${id}`,
    ...overrides,
  };
}

test("isStoredGifFavorite accepts the normalized GIF shape", () => {
  assert.equal(isStoredGifFavorite(gif("a")), true);
  assert.equal(isStoredGifFavorite(gif("a", { poster: null })), true);
});

test("isStoredGifFavorite rejects malformed records", () => {
  assert.equal(isStoredGifFavorite(null), false);
  assert.equal(isStoredGifFavorite("gif"), false);
  assert.equal(isStoredGifFavorite(gif("")), false);
  assert.equal(isStoredGifFavorite(gif("a", { slug: "" })), false);
  assert.equal(isStoredGifFavorite(gif("a", { title: 7 })), false);
  assert.equal(isStoredGifFavorite(gif("a", { poster: undefined })), false);
  assert.equal(
    isStoredGifFavorite(
      gif("a", { original: { url: "http://x", width: 1, height: 1, size: 1 } }),
    ),
    false,
    "plain http URLs are rejected",
  );
  assert.equal(
    isStoredGifFavorite(
      gif("a", {
        original: { url: `${MEDIA}/a.gif`, width: "640", height: 360, size: 1 },
      }),
    ),
    false,
    "string dimensions are rejected after normalization",
  );
  assert.equal(
    isStoredGifFavorite(
      gif("a", {
        original: { url: `${MEDIA}/a.gif`, width: 640, height: 360 },
      }),
    ),
    false,
    "the sendable asset needs a size",
  );
  assert.equal(
    isStoredGifFavorite(
      gif("a", { preview: { url: `${MEDIA}/a.webp`, width: 0, height: 10 } }),
    ),
    false,
  );
});

test("parseGifFavorites tolerates missing, invalid, and non-array payloads", () => {
  assert.deepEqual(parseGifFavorites(null), []);
  assert.deepEqual(parseGifFavorites(""), []);
  assert.deepEqual(parseGifFavorites("{not json"), []);
  assert.deepEqual(parseGifFavorites('{"id":"a"}'), []);
  assert.deepEqual(parseGifFavorites("42"), []);
});

test("parseGifFavorites keeps valid entries, drops bad ones, dedupes by id", () => {
  const stored = JSON.stringify([
    gif("a"),
    { id: "broken" },
    gif("b", { poster: null }),
    gif("a", { title: "Duplicate" }),
  ]);
  const parsed = parseGifFavorites(stored);
  assert.deepEqual(
    parsed.map((entry) => entry.id),
    ["a", "b"],
  );
  assert.equal(parsed[0].title, "Party a");
  assert.equal(parsed[1].poster, null);
});

test("parseGifFavorites strips unknown fields and caps the list", () => {
  const stored = JSON.stringify(
    Array.from({ length: MAX_GIF_FAVORITES + 5 }, (_, index) => ({
      ...gif(`g${index}`),
      extra: "ignored",
    })),
  );
  const parsed = parseGifFavorites(stored);
  assert.equal(parsed.length, MAX_GIF_FAVORITES);
  assert.equal("extra" in parsed[0], false);
  assert.deepEqual(Object.keys(parsed[0]).sort(), [
    "id",
    "original",
    "poster",
    "preview",
    "slug",
    "title",
  ]);
});

test("toggleGifFavorite adds to the front and removes when present", () => {
  const a = gif("a");
  const b = gif("b");
  const afterA = toggleGifFavorite([], a);
  assert.deepEqual(afterA, [a]);
  const afterB = toggleGifFavorite(afterA, b);
  assert.deepEqual(
    afterB.map((entry) => entry.id),
    ["b", "a"],
    "newest favorite renders first",
  );
  assert.equal(isGifFavorite(afterB, "a"), true);
  const removed = toggleGifFavorite(afterB, a);
  assert.deepEqual(
    removed.map((entry) => entry.id),
    ["b"],
  );
  assert.equal(isGifFavorite(removed, "a"), false);
});

test("toggleGifFavorite does not mutate its input", () => {
  const original = [gif("a")];
  const snapshot = JSON.stringify(original);
  toggleGifFavorite(original, gif("b"));
  toggleGifFavorite(original, gif("a"));
  assert.equal(JSON.stringify(original), snapshot);
});

test("toggleGifFavorite drops the oldest entry past the cap", () => {
  const full = Array.from({ length: MAX_GIF_FAVORITES }, (_, index) =>
    gif(`g${index}`),
  );
  const next = toggleGifFavorite(full, gif("newest"));
  assert.equal(next.length, MAX_GIF_FAVORITES);
  assert.equal(next[0].id, "newest");
  assert.equal(isGifFavorite(next, `g${MAX_GIF_FAVORITES - 1}`), false);
  assert.equal(isGifFavorite(next, "g0"), true);
});

test("filterGifFavorites matches titles case-insensitively and trims", () => {
  const favorites = [
    gif("a", { title: "Party Parrot" }),
    gif("b", { title: "Thumbs Up" }),
    gif("c", { title: "Dance party" }),
  ];
  assert.deepEqual(
    filterGifFavorites(favorites, "  PARTY ").map((entry) => entry.id),
    ["a", "c"],
  );
  assert.deepEqual(
    filterGifFavorites(favorites, "").map((entry) => entry.id),
    ["a", "b", "c"],
  );
  assert.deepEqual(filterGifFavorites(favorites, "nope"), []);
});
