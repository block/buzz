import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_AVATAR_PIXEL_SIZE,
  downscaleAvatarDataUrl,
} from "./avatarDownscale.ts";

/**
 * Builds the decode/encode seam with a recording target. `encode` maps a
 * requested mime type to the data URL the surface would return, standing in for
 * a webview's `toDataURL`.
 */
function fakeDeps({ source, encode, targetForSize = () => true }) {
  const calls = { closed: 0, draws: [], targets: [] };
  const bitmap = {
    ...source,
    close: () => {
      calls.closed += 1;
    },
  };
  return {
    calls,
    deps: {
      createTarget: (size) => {
        calls.targets.push(size);
        if (!targetForSize(size)) return null;
        return {
          drawImage: (...args) => {
            calls.draws.push(args);
          },
          toDataURL: (type, quality) => encode(type, quality, size),
        };
      },
      decode: async () => (source ? bitmap : null),
    },
  };
}

/** A data URL of `length` characters carrying `type`'s prefix. */
function dataUrlOfLength(type, length) {
  const prefix = `data:${type};base64,`;
  return prefix + "A".repeat(Math.max(0, length - prefix.length));
}

const BLOB = /** @type {Blob} */ ({});

test("a large avatar is redrawn at the target size before it is encoded", async () => {
  const { calls, deps } = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 4_000),
    source: { height: 512, width: 512 },
  });

  const result = await downscaleAvatarDataUrl(BLOB, { deps });

  assert.ok(result.startsWith("data:image/webp;base64,"));
  // Binds the fix: the 512px source must reach the encoder as a
  // DEFAULT_AVATAR_PIXEL_SIZE square. Encoding at full size fails here.
  assert.deepEqual(calls.targets, [DEFAULT_AVATAR_PIXEL_SIZE]);
  const [, sx, sy, sw, sh, dx, dy, dw, dh] = calls.draws[0];
  assert.deepEqual(
    { dh, dw, dx, dy },
    {
      dh: DEFAULT_AVATAR_PIXEL_SIZE,
      dw: DEFAULT_AVATAR_PIXEL_SIZE,
      dx: 0,
      dy: 0,
    },
  );
  assert.deepEqual({ sh, sw, sx, sy }, { sh: 512, sw: 512, sx: 0, sy: 0 });
});

test("a non-square avatar is center-cropped to its shorter side", async () => {
  const { calls, deps } = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 100),
    source: { height: 100, width: 300 },
  });

  await downscaleAvatarDataUrl(BLOB, { deps, pixelSize: 64 });

  const [, sx, sy, sw, sh] = calls.draws[0];
  assert.deepEqual({ sh, sw, sx, sy }, { sh: 100, sw: 100, sx: 100, sy: 0 });
});

test("an avatar smaller than the target is not upscaled", async () => {
  const { calls, deps } = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 100),
    source: { height: 32, width: 32 },
  });

  await downscaleAvatarDataUrl(BLOB, { deps, pixelSize: 96 });

  assert.deepEqual(calls.targets, [32]);
});

test("an unsupported webp encoder falls through to jpeg", async () => {
  // Safari returns a PNG rather than failing when it cannot encode WebP.
  const { deps } = fakeDeps({
    encode: (type) =>
      type === "image/webp"
        ? dataUrlOfLength("image/png", 500)
        : dataUrlOfLength(type, 500),
    source: { height: 200, width: 200 },
  });

  const result = await downscaleAvatarDataUrl(BLOB, { deps });

  assert.ok(result.startsWith("data:image/jpeg;base64,"));
});

test("an encoding over the cap is rejected and the next one is tried", async () => {
  const { deps } = fakeDeps({
    encode: (type) =>
      dataUrlOfLength(type, type === "image/webp" ? 9_000 : 1_000),
    source: { height: 200, width: 200 },
  });

  const result = await downscaleAvatarDataUrl(BLOB, {
    deps,
    maxDataUrlLength: 2_000,
  });

  assert.ok(result.startsWith("data:image/jpeg;base64,"));
});

test("an avatar that fits no encoding degrades to null rather than a broken image", async () => {
  const { deps } = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 50_000),
    source: { height: 200, width: 200 },
  });

  const result = await downscaleAvatarDataUrl(BLOB, {
    deps,
    maxDataUrlLength: 16 * 1_024,
  });

  assert.equal(result, null);
});

test("a throwing encoder is skipped instead of failing the avatar", async () => {
  const { deps } = fakeDeps({
    encode: (type) => {
      if (type === "image/webp") throw new Error("encoder unavailable");
      return dataUrlOfLength(type, 500);
    },
    source: { height: 200, width: 200 },
  });

  assert.ok(
    (await downscaleAvatarDataUrl(BLOB, { deps })).startsWith(
      "data:image/jpeg;base64,",
    ),
  );
});

test("an undecodable blob, a zero-size source, and a missing surface all yield null", async () => {
  const undecodable = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 100),
    source: null,
  });
  assert.equal(
    await downscaleAvatarDataUrl(BLOB, { deps: undecodable.deps }),
    null,
  );

  const empty = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 100),
    source: { height: 0, width: 0 },
  });
  assert.equal(await downscaleAvatarDataUrl(BLOB, { deps: empty.deps }), null);

  const noSurface = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 100),
    source: { height: 64, width: 64 },
    targetForSize: () => false,
  });
  assert.equal(
    await downscaleAvatarDataUrl(BLOB, { deps: noSurface.deps }),
    null,
  );
});

test("the decoded bitmap is released on both the success and failure paths", async () => {
  const ok = fakeDeps({
    encode: (type) => dataUrlOfLength(type, 100),
    source: { height: 64, width: 64 },
  });
  await downscaleAvatarDataUrl(BLOB, { deps: ok.deps });
  assert.equal(ok.calls.closed, 1);

  const failed = fakeDeps({
    encode: () => {
      throw new Error("boom");
    },
    source: { height: 64, width: 64 },
  });
  await downscaleAvatarDataUrl(BLOB, { deps: failed.deps });
  assert.equal(failed.calls.closed, 1);
});
