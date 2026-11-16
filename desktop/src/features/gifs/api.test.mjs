import assert from "node:assert/strict";
import test from "node:test";

import {
  giphyGifAttachment,
  giphyGifFilename,
  normalizeGiphyGifs,
  relayGiphyCapability,
} from "./api.ts";

const MEDIA = "https://media1.giphy.com/media/abc123";

const DOWNSIZED = {
  url: `${MEDIA}/giphy-downsized.gif`,
  width: "640",
  height: "360",
  size: "42",
};

const ORIGINAL = {
  url: `${MEDIA}/giphy.gif`,
  width: "1280",
  height: "720",
  size: "900000",
  webp: `${MEDIA}/giphy.webp`,
  webp_size: "500000",
};

const FIXED_WIDTH = {
  url: `${MEDIA}/200w.gif`,
  width: "200",
  height: "112",
  size: "7000",
  webp: `${MEDIA}/200w.webp`,
  webp_size: "3000",
};

const FIXED_WIDTH_SMALL = {
  url: `${MEDIA}/100w.gif`,
  width: "100",
  height: "56",
  size: "2000",
  webp: `${MEDIA}/100w.webp`,
  webp_size: "900",
};

const FIXED_WIDTH_STILL = {
  url: `${MEDIA}/200w_s.gif`,
  width: "200",
  height: "112",
};

function rawGif(overrides = {}) {
  return {
    type: "gif",
    id: "abc123",
    slug: "ship-it-abc123",
    title: "  Ship it  ",
    images: {
      downsized: DOWNSIZED,
      original: ORIGINAL,
      fixed_width: FIXED_WIDTH,
      fixed_width_small: FIXED_WIDTH_SMALL,
      fixed_width_still: FIXED_WIDTH_STILL,
    },
    ...overrides,
  };
}

const NORMALIZED = {
  id: "abc123",
  original: {
    url: DOWNSIZED.url,
    width: 640,
    height: 360,
    size: 42,
  },
  poster: { url: FIXED_WIDTH_STILL.url, width: 200, height: 112 },
  preview: { url: FIXED_WIDTH.webp, width: 200, height: 112 },
  slug: "ship-it-abc123",
  title: "Ship it",
};

test("normalizeGiphyGifs parses string dimensions and picks the preferred renditions", () => {
  const gifs = normalizeGiphyGifs([rawGif()]);

  assert.deepEqual(gifs, [NORMALIZED]);
  assert.equal(typeof gifs[0].id, "string");
  assert.equal(typeof gifs[0].original.width, "number");
  assert.equal(typeof gifs[0].original.size, "number");
  assert.equal(typeof gifs[0].preview.height, "number");
});

test("normalizeGiphyGifs rejects non-integer or non-positive dimensions", () => {
  for (const bad of ["", "0", "-1", "320px", "320.5", " 320", 320, null]) {
    const gifs = normalizeGiphyGifs([
      rawGif({
        images: {
          downsized: { ...DOWNSIZED, width: bad },
          fixed_width: FIXED_WIDTH,
        },
      }),
    ]);
    // The downsized rendition is incomplete, so fixed_width is sent instead.
    assert.equal(gifs.length, 1, `width ${JSON.stringify(bad)}`);
    assert.equal(gifs[0].original.url, FIXED_WIDTH.url);
  }
});

test("normalizeGiphyGifs sendable asset falls back downsized -> original -> fixed_width", () => {
  const [fromOriginal] = normalizeGiphyGifs([
    rawGif({ images: { original: ORIGINAL, fixed_width: FIXED_WIDTH } }),
  ]);
  assert.deepEqual(fromOriginal.original, {
    url: ORIGINAL.url,
    width: 1280,
    height: 720,
    size: 900000,
  });

  const [fromFixedWidth] = normalizeGiphyGifs([
    rawGif({ images: { fixed_width: FIXED_WIDTH } }),
  ]);
  assert.deepEqual(fromFixedWidth.original, {
    url: FIXED_WIDTH.url,
    width: 200,
    height: 112,
    size: 7000,
  });

  // A sendable asset needs a size; a downsized rendition without one is skipped.
  const [sizeless] = normalizeGiphyGifs([
    rawGif({
      images: {
        downsized: { ...DOWNSIZED, size: undefined },
        original: ORIGINAL,
      },
    }),
  ]);
  assert.equal(sizeless.original.url, ORIGINAL.url);

  // Size zero is still a valid size.
  const [zeroSize] = normalizeGiphyGifs([
    rawGif({ images: { downsized: { ...DOWNSIZED, size: "0" } } }),
  ]);
  assert.equal(zeroSize.original.size, 0);
});

test("normalizeGiphyGifs preview falls back fixed_width webp -> gif -> small webp -> small gif -> original", () => {
  const previewFor = (images) =>
    normalizeGiphyGifs([
      rawGif({ images: { downsized: DOWNSIZED, ...images } }),
    ])[0].preview;

  assert.deepEqual(previewFor({ fixed_width: FIXED_WIDTH }), {
    url: FIXED_WIDTH.webp,
    width: 200,
    height: 112,
  });
  assert.deepEqual(
    previewFor({ fixed_width: { ...FIXED_WIDTH, webp: undefined } }),
    { url: FIXED_WIDTH.url, width: 200, height: 112 },
  );
  assert.deepEqual(previewFor({ fixed_width_small: FIXED_WIDTH_SMALL }), {
    url: FIXED_WIDTH_SMALL.webp,
    width: 100,
    height: 56,
  });
  assert.deepEqual(
    previewFor({ fixed_width_small: { ...FIXED_WIDTH_SMALL, webp: "" } }),
    { url: FIXED_WIDTH_SMALL.url, width: 100, height: 56 },
  );
  assert.deepEqual(previewFor({}), {
    url: DOWNSIZED.url,
    width: 640,
    height: 360,
  });
});

test("normalizeGiphyGifs poster falls back through the *_still renditions", () => {
  const posterFor = (images) =>
    normalizeGiphyGifs([
      rawGif({ images: { downsized: DOWNSIZED, ...images } }),
    ])[0].poster;
  const still = (name) => ({
    url: `${MEDIA}/${name}.gif`,
    width: "10",
    height: "5",
  });

  assert.equal(
    posterFor({
      fixed_width_still: still("a"),
      fixed_width_small_still: still("b"),
      downsized_still: still("c"),
      original_still: still("d"),
    }).url,
    `${MEDIA}/a.gif`,
  );
  assert.equal(
    posterFor({
      fixed_width_small_still: still("b"),
      downsized_still: still("c"),
      original_still: still("d"),
    }).url,
    `${MEDIA}/b.gif`,
  );
  assert.equal(
    posterFor({ downsized_still: still("c"), original_still: still("d") }).url,
    `${MEDIA}/c.gif`,
  );
  assert.equal(posterFor({ original_still: still("d") }).url, `${MEDIA}/d.gif`);
  // A still rendition with a bad dimension is skipped in favor of the next one.
  assert.equal(
    posterFor({
      fixed_width_still: { ...still("a"), height: "0" },
      original_still: still("d"),
    }).url,
    `${MEDIA}/d.gif`,
  );
});

test("normalizeGiphyGifs poster is null when no *_still rendition is usable", () => {
  const [gif] = normalizeGiphyGifs([
    rawGif({ images: { downsized: DOWNSIZED, fixed_width: FIXED_WIDTH } }),
  ]);
  assert.equal(gif.poster, null);

  const [badStill] = normalizeGiphyGifs([
    rawGif({
      images: {
        downsized: DOWNSIZED,
        fixed_width_still: { url: "", width: "200", height: "112" },
      },
    }),
  ]);
  assert.equal(badStill.poster, null);
});

test("normalizeGiphyGifs skips items with no sendable asset", () => {
  const gifs = normalizeGiphyGifs([
    // No renditions at all.
    rawGif({ images: {} }),
    // Only still renditions.
    rawGif({ images: { fixed_width_still: FIXED_WIDTH_STILL } }),
    // Only a small rendition, which is never sendable.
    rawGif({ images: { fixed_width_small: FIXED_WIDTH_SMALL } }),
    // Candidates exist but every one is incomplete.
    rawGif({
      images: {
        downsized: { ...DOWNSIZED, url: "" },
        original: { ...ORIGINAL, height: "abc" },
        fixed_width: { ...FIXED_WIDTH, size: "-1" },
      },
    }),
  ]);

  assert.deepEqual(gifs, []);
});

test("normalizeGiphyGifs skips non-gif, id-less, slug-less and image-less items", () => {
  const gifs = normalizeGiphyGifs([
    rawGif({ type: "sticker" }),
    rawGif({ type: undefined }),
    rawGif({ id: "" }),
    rawGif({ id: 7 }),
    rawGif({ id: undefined }),
    rawGif({ slug: "" }),
    rawGif({ slug: undefined }),
    rawGif({ images: undefined }),
    rawGif({ images: null }),
    rawGif({ images: "nope" }),
  ]);

  assert.deepEqual(gifs, []);
});

test("normalizeGiphyGifs title falls back to alt_text, then GIF", () => {
  const titles = normalizeGiphyGifs([
    rawGif({ title: "  Party  ", alt_text: "A parrot" }),
    rawGif({ title: "   ", alt_text: "  A parrot  " }),
    rawGif({ title: "", alt_text: "" }),
    rawGif({ title: undefined, alt_text: undefined }),
    rawGif({ title: 12, alt_text: "   " }),
  ]).map((gif) => gif.title);

  assert.deepEqual(titles, ["Party", "A parrot", "GIF", "GIF", "GIF"]);
});

test("relayGiphyCapability requires buzz-gif, the giphy provider and a safe search path", () => {
  assert.deepEqual(
    relayGiphyCapability({
      gif: { provider: "giphy", search: "/gifs/search" },
      supported_extensions: ["nip-er", "buzz-gif"],
    }),
    { searchPath: "/gifs/search" },
  );
  assert.equal(relayGiphyCapability({}), null);
  // The buzz-gif extension must be advertised.
  assert.equal(
    relayGiphyCapability({
      gif: { provider: "giphy", search: "/gifs/search" },
      supported_extensions: ["nip-er"],
    }),
    null,
  );
  assert.equal(
    relayGiphyCapability({
      gif: { provider: "giphy", search: "/gifs/search" },
    }),
    null,
  );
  // Any other provider value, including the retired one, is not supported.
  for (const provider of ["another", "tenor", "", undefined, "GIPHY"]) {
    assert.equal(
      relayGiphyCapability({
        gif: { provider, search: "/gifs/search" },
        supported_extensions: ["buzz-gif"],
      }),
      null,
      `provider ${JSON.stringify(provider)}`,
    );
  }
  for (const path of [
    "https://attacker.example/search",
    "//attacker.example/search",
    "/\\attacker.example/search",
    "/%5c%5cattacker.example/search",
    "/gifs/../admin",
    "/gifs/%2e%2e/admin",
    "/gifs/search?redirect=https://attacker.example",
    "/gifs/search#fragment",
    "",
    undefined,
  ]) {
    assert.equal(
      relayGiphyCapability({
        gif: { provider: "giphy", search: path },
        supported_extensions: ["buzz-gif"],
      }),
      null,
      `path ${JSON.stringify(path)}`,
    );
  }
});

test("relayGiphyCapability does not require or expose a share path", () => {
  const info = {
    gif: {
      provider: "giphy",
      search: "/gifs/search",
      share: "/gifs/share",
    },
    supported_extensions: ["buzz-gif"],
  };

  assert.deepEqual(relayGiphyCapability(info), { searchPath: "/gifs/search" });
  // An unsafe legacy share path is ignored rather than disabling GIFs.
  assert.deepEqual(
    relayGiphyCapability({
      ...info,
      gif: { ...info.gif, share: "https://attacker.example/share" },
    }),
    { searchPath: "/gifs/search" },
  );
});

test("giphyGifFilename sanitizes provider slugs", () => {
  const gif = {
    ...NORMALIZED,
    slug: "  That's a wrap!  ",
    title: "That's a wrap",
  };

  assert.equal(giphyGifFilename(gif), "that-s-a-wrap.gif");
  assert.equal(giphyGifFilename({ ...gif, slug: "!!!" }), "giphy-gif.gif");
});

test("giphyGifAttachment references GIPHY media without an uploaded copy", () => {
  const attachment = giphyGifAttachment(NORMALIZED);

  assert.deepEqual(attachment, {
    dim: "640x360",
    displayLabel: "Ship it",
    filename: "ship-it-abc123.gif",
    sha256: "",
    size: 42,
    type: "image/gif",
    uploaded: 0,
    url: DOWNSIZED.url,
  });
});
