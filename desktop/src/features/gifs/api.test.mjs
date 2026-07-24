import assert from "node:assert/strict";
import test from "node:test";

import { klipyGifFilename, normalizeKlipyGifs } from "./api.ts";

const GIF_ASSET = {
  url: "https://static.klipy.com/example.gif",
  width: 640,
  height: 360,
  size: 42,
};

test("normalizeKlipyGifs selects a compact preview and medium GIF", () => {
  const [gif] = normalizeKlipyGifs([
    {
      id: 7,
      title: "  Ship it  ",
      slug: "ship-it",
      type: "gif",
      file: {
        md: { gif: GIF_ASSET },
        sm: {
          webp: {
            url: "https://static.klipy.com/preview.webp",
            width: 220,
            height: 124,
            size: 12,
          },
        },
      },
    },
  ]);

  assert.equal(gif.title, "Ship it");
  assert.equal(gif.original.url, GIF_ASSET.url);
  assert.equal(gif.preview.url, "https://static.klipy.com/preview.webp");
});

test("normalizeKlipyGifs omits ads and malformed file records", () => {
  const gifs = normalizeKlipyGifs([
    { id: 1, slug: "ad", type: "ad" },
    { id: 2, slug: "missing", type: "gif", file: {} },
  ]);

  assert.deepEqual(gifs, []);
});

test("klipyGifFilename sanitizes provider slugs", () => {
  const filename = klipyGifFilename({
    id: 1,
    original: GIF_ASSET,
    preview: GIF_ASSET,
    slug: "  That's a wrap!  ",
    title: "That's a wrap",
  });

  assert.equal(filename, "that-s-a-wrap.gif");
});
