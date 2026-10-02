import assert from "node:assert/strict";
import test from "node:test";

import {
  klipyGifAttachment,
  klipyGifFilename,
  normalizeKlipyGifs,
  relayKlipyCapability,
} from "./api.ts";

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
  assert.equal(gif.poster, null);
});

test("normalizeKlipyGifs carries a static jpg poster when present", () => {
  const [gif] = normalizeKlipyGifs([
    {
      id: 8,
      title: "Static poster",
      slug: "static-poster",
      type: "gif",
      file: {
        md: { gif: GIF_ASSET },
        sm: {
          jpg: {
            url: "https://static.klipy.com/poster.jpg",
            width: 220,
            height: 124,
            size: 8,
          },
        },
      },
    },
  ]);

  assert.equal(gif.poster?.url, "https://static.klipy.com/poster.jpg");
});

test("normalizeKlipyGifs omits ads and malformed file records", () => {
  const gifs = normalizeKlipyGifs([
    { id: 1, slug: "ad", type: "ad" },
    { id: 2, slug: "missing", type: "gif", file: {} },
  ]);

  assert.deepEqual(gifs, []);
});

test("relayKlipyCapability requires safe search and share endpoints", () => {
  assert.deepEqual(
    relayKlipyCapability({
      gif: {
        provider: "klipy",
        search: "/gifs/search",
        share: "/gifs/share",
      },
      supported_extensions: ["nip-er", "buzz-gif"],
    }),
    { searchPath: "/gifs/search", sharePath: "/gifs/share" },
  );
  assert.equal(relayKlipyCapability({}), null);
  assert.equal(
    relayKlipyCapability({
      gif: {
        provider: "another",
        search: "/gifs/search",
        share: "/gifs/share",
      },
      supported_extensions: ["buzz-gif"],
    }),
    null,
  );
  assert.equal(
    relayKlipyCapability({
      gif: { provider: "klipy", search: "/gifs/search" },
      supported_extensions: ["buzz-gif"],
    }),
    null,
  );
  for (const path of [
    "https://attacker.example/search",
    "//attacker.example/search",
    "/\\attacker.example/search",
    "/%5c%5cattacker.example/search",
    "/gifs/../admin",
    "/gifs/%2e%2e/admin",
    "/gifs/search?redirect=https://attacker.example",
    "/gifs/search#fragment",
  ]) {
    assert.equal(
      relayKlipyCapability({
        gif: {
          provider: "klipy",
          search: path,
          share: "/gifs/share",
        },
        supported_extensions: ["buzz-gif"],
      }),
      null,
    );
    assert.equal(
      relayKlipyCapability({
        gif: {
          provider: "klipy",
          search: "/gifs/search",
          share: path,
        },
        supported_extensions: ["buzz-gif"],
      }),
      null,
    );
  }
});

test("klipyGifFilename sanitizes provider slugs", () => {
  const gif = {
    id: 1,
    original: GIF_ASSET,
    poster: null,
    preview: GIF_ASSET,
    slug: "  That's a wrap!  ",
    title: "That's a wrap",
  };

  const filename = klipyGifFilename(gif);

  assert.equal(filename, "that-s-a-wrap.gif");
});

test("klipyGifAttachment references KLIPY media without an uploaded copy", () => {
  const attachment = klipyGifAttachment({
    id: 1,
    original: GIF_ASSET,
    poster: null,
    preview: GIF_ASSET,
    slug: "ship-it",
    title: "Ship it",
  });

  assert.deepEqual(attachment, {
    dim: "640x360",
    displayLabel: "Ship it",
    filename: "ship-it.gif",
    sha256: "",
    size: 42,
    type: "image/gif",
    uploaded: 0,
    url: GIF_ASSET.url,
  });
});
