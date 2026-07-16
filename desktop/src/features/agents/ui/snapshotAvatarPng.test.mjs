import assert from "node:assert/strict";
import test from "node:test";

import { resolveSnapshotAvatarPng } from "./snapshotAvatarPng.ts";

const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);

test("resolveSnapshotAvatarPng: relay media URL becomes PNG data URL", async () => {
  const result = await resolveSnapshotAvatarPng(
    "https://relay.example/media/avatar.png",
    {
      fetchBytes: async (url) => {
        assert.equal(url, "https://relay.example/media/avatar.png");
        return PNG_BYTES;
      },
    },
  );

  assert.equal(result, "data:image/png;base64,iVBORw==");
});

test("resolveSnapshotAvatarPng: emoji SVG is rasterized onto a canvas", async () => {
  const draws = [];
  const image = {
    src: "",
    decode: async () => {},
  };
  const canvas = {
    width: 0,
    height: 0,
    getContext: () => ({
      drawImage: (...args) => draws.push(args),
    }),
    toDataURL: (type) => {
      assert.equal(type, "image/png");
      return "data:image/png;base64,cmFzdGVyaXplZA==";
    },
  };

  const emojiSvg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" rx="256" fill="#ffcc00"/><text>✨</text></svg>';
  const result = await resolveSnapshotAvatarPng(
    `data:image/svg+xml,${encodeURIComponent(emojiSvg)}`,
    {
      createCanvas: () => canvas,
      createImage: () => image,
    },
  );

  assert.equal(result, "data:image/png;base64,cmFzdGVyaXplZA==");
  const rasterizedSvg = decodeURIComponent(image.src.split(",", 2)[1]);
  assert.match(
    rasterizedSvg,
    /<rect width="512" height="512" fill="#ffcc00"\/>/u,
  );
  assert.doesNotMatch(rasterizedSvg, /rx="256"/u);
  assert.equal(canvas.width, 512);
  assert.equal(canvas.height, 512);
  assert.deepEqual(draws, [[image, 0, 0, 512, 512]]);
});

test("resolveSnapshotAvatarPng: failed media fetches and malformed URLs return undefined", async () => {
  let fetchCalled = false;
  const dependencies = {
    fetchBytes: async () => {
      fetchCalled = true;
      throw new Error("external URL rejected by Rust validation");
    },
  };

  assert.equal(
    await resolveSnapshotAvatarPng(
      "https://external.example/avatar.png",
      dependencies,
    ),
    undefined,
  );
  assert.equal(
    await resolveSnapshotAvatarPng("not a URL", dependencies),
    undefined,
  );
  assert.equal(fetchCalled, true);
});
