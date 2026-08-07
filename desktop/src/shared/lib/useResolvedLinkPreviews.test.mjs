import assert from "node:assert/strict";
import test from "node:test";

import {
  __linkPreviewMetadataTest,
  resolveLinkPreview,
} from "./useResolvedLinkPreviews.ts";

const preview = {
  kind: "generic-link",
  href: "https://example.com/story",
  provider: "example.com",
  title: "example.com/story",
  typeLabel: "link",
};

function metadata(overrides = {}) {
  return {
    title: "A story",
    siteName: "Example",
    description: "Story description",
    imageDataUrl: null,
    imageDomain: null,
    imageFetchState: "none",
    imageRetryAfterMs: null,
    ...overrides,
  };
}

test("pending metadata reserves the image treatment", () => {
  assert.deepEqual(resolveLinkPreview(preview, undefined), {
    ...preview,
    imageState: "pending",
  });
});

test("resolved image metadata keeps the reserved image treatment", () => {
  const resolved = resolveLinkPreview(preview, {
    title: "A story",
    siteName: "Example",
    imageDataUrl: "data:image/jpeg;base64,abc",
    imageDomain: "cdn.example.com",
  });

  assert.equal(resolved.imageState, "image");
  assert.equal(resolved.provider, "Example");
  assert.equal(resolved.imageDomain, "cdn.example.com");
});

test("resolved metadata without a complete image collapses to the compact treatment", () => {
  const resolved = resolveLinkPreview(preview, {
    title: "A story",
    siteName: "Example",
    imageDataUrl: null,
    imageDomain: null,
  });

  assert.equal(resolved.imageState, "none");
  assert.equal(resolved.imageDataUrl, null);
  assert.equal(resolved.imageDomain, null);
});

test("transient and rejected image fetches use the stable fallback treatment", () => {
  const transient = resolveLinkPreview(
    preview,
    metadata({
      imageFetchState: "transient_failure",
      imageRetryAfterMs: 900_000,
    }),
  );
  const rejected = resolveLinkPreview(
    preview,
    metadata({ imageFetchState: "rejected" }),
  );

  assert.equal(transient.imageState, "fallback");
  assert.equal(rejected.imageState, "fallback");
});

test("metadata cache keys deduplicate URL fragments", () => {
  assert.equal(
    __linkPreviewMetadataTest.metadataCacheKey(
      "https://github.com/block/buzz/pull/3834#issuecomment-1",
    ),
    "https://github.com/block/buzz/pull/3834",
  );
});

test("transient metadata expires at the server retry boundary", () => {
  assert.equal(
    __linkPreviewMetadataTest.metadataExpiry(
      metadata({
        imageFetchState: "transient_failure",
        imageRetryAfterMs: 900_000,
      }),
      1_000,
    ),
    901_000,
  );
});

test("metadata loader retries transient images after the server cooldown", async () => {
  let now = 1_000;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: async () => {
      calls += 1;
      return calls === 1
        ? metadata({
            imageFetchState: "transient_failure",
            imageRetryAfterMs: 10_000,
          })
        : metadata({
            imageDataUrl: "data:image/jpeg;base64,abc",
            imageDomain: "images.example.com",
            imageFetchState: "image",
          });
    },
    now: () => now,
  });

  assert.equal(
    (await loader.load(preview.href)).metadata?.imageFetchState,
    "transient_failure",
  );
  assert.equal(calls, 1);

  now += 10_000;
  assert.equal(
    (await loader.load(preview.href)).metadata?.imageFetchState,
    "image",
  );
  assert.equal(calls, 2);
});

test("metadata loader retries rejected requests after the negative-cache TTL", async () => {
  let now = 1_000;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: async () => {
      calls += 1;
      if (calls === 1) throw new Error("temporary failure");
      return metadata();
    },
    now: () => now,
  });

  assert.equal((await loader.load(preview.href)).metadata, null);
  assert.equal((await loader.load(preview.href)).metadata, null);
  assert.equal(calls, 1);

  now += 5 * 60_000;
  assert.deepEqual((await loader.load(preview.href)).metadata, metadata());
  assert.equal(calls, 2);
});

test("metadata loader coalesces fragment variants and bounds concurrency", async () => {
  let active = 0;
  let maxActive = 0;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    concurrency: 2,
    fetcher: async () => {
      calls += 1;
      active += 1;
      maxActive = Math.max(maxActive, active);
      await new Promise((resolve) => setImmediate(resolve));
      active -= 1;
      return metadata();
    },
  });

  await Promise.all([
    loader.load("https://example.com/one#first"),
    loader.load("https://example.com/one#second"),
    loader.load("https://example.com/two"),
    loader.load("https://example.com/three"),
  ]);

  assert.equal(calls, 3);
  assert.equal(maxActive, 2);
});
