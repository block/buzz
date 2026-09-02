import assert from "node:assert/strict";
import test from "node:test";

import {
  CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET,
  selectAvatarsWithinBudget,
  splitAvatarDataUrl,
  toCanvasAvatarUploads,
} from "./canvasAvatars.ts";
import { PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES } from "./projectCanvasProtocol.ts";

/** A data URL of exactly `length` characters. */
function avatar(length) {
  const prefix = "data:image/webp;base64,";
  return prefix + "A".repeat(Math.max(0, length - prefix.length));
}

function totalLength(dataUrls) {
  return dataUrls.reduce((sum, value) => sum + (value?.length ?? 0), 0);
}

test("the combined avatar budget leaves room inside one rpc message", () => {
  // The whole response — avatars, names, pubkeys, envelope — must fit the port
  // ceiling, so the avatar share has to stay strictly under it.
  assert.ok(
    CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET < PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES,
  );
});

test("avatars are kept in order until the budget is spent", () => {
  const rows = [avatar(10_000), avatar(10_000), avatar(10_000)];

  const selected = selectAvatarsWithinBudget(rows, 25_000);

  assert.deepEqual(
    selected.map((value) => value !== null),
    [true, true, false],
  );
  assert.ok(totalLength(selected) <= 25_000);
});

test("a lookup that would overrun the port ceiling is trimmed under it", () => {
  // 32 people is the lookup maximum; at the per-avatar cap they would total
  // 512 KiB and the frame would get `too-large` instead of the result.
  const rows = Array.from({ length: 32 }, () => avatar(16 * 1_024));

  const selected = selectAvatarsWithinBudget(rows);

  assert.ok(totalLength(selected) <= CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET);
  assert.ok(totalLength(selected) < PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES);
  assert.ok(selected.some((value) => value !== null));
});

test("a single avatar larger than the whole budget is dropped", () => {
  const selected = selectAvatarsWithinBudget([avatar(50_000)], 40_000);

  assert.deepEqual(selected, [null]);
});

test("missing avatars pass through without consuming budget", () => {
  const selected = selectAvatarsWithinBudget(
    [null, avatar(30_000), null, avatar(9_000)],
    40_000,
  );

  assert.deepEqual(
    selected.map((value) => value !== null),
    [false, true, false, true],
  );
});

test("the result is index-aligned with its input so rows can be zipped back", () => {
  const rows = [avatar(100), null, avatar(100)];

  assert.equal(selectAvatarsWithinBudget(rows).length, rows.length);
});

// --- Publishing ------------------------------------------------------------
// Published avatars leave the RPC payload entirely: the frame loads them from
// `__buzz/avatar/<pubkey>`. These bind the split the backend's decoder expects.

test("a base64 data url splits into the media type and payload", () => {
  assert.deepEqual(splitAvatarDataUrl("data:image/webp;base64,QUJD"), {
    contentType: "image/webp",
    data: "QUJD",
  });
});

test("anything that is not a base64 image data url yields null", () => {
  // A percent-encoded data URL carries no base64 payload; forwarding its text
  // as `data` would have the backend reject the whole batch.
  assert.equal(splitAvatarDataUrl("data:image/svg+xml,%3Csvg%2F%3E"), null);
  assert.equal(splitAvatarDataUrl("data:text/plain;base64,QUJD"), null);
  assert.equal(splitAvatarDataUrl("https://example.test/a.png"), null);
  assert.equal(splitAvatarDataUrl("data:image/png;base64,"), null);
  assert.equal(splitAvatarDataUrl(""), null);
});

test("uploads carry the pubkey the frame will request the avatar by", () => {
  assert.deepEqual(
    toCanvasAvatarUploads([
      { dataUrl: "data:image/webp;base64,QUJD", pubkey: "ab".repeat(32) },
    ]),
    [
      {
        contentType: "image/webp",
        data: "QUJD",
        pubkey: "ab".repeat(32),
      },
    ],
  );
});

test("people without a usable avatar are skipped, not sent as null", () => {
  // The backend rejects a malformed batch wholesale, so one unusable entry
  // must not cost everyone else their picture.
  const uploads = toCanvasAvatarUploads([
    { dataUrl: null, pubkey: "aa".repeat(32) },
    { dataUrl: "https://example.test/a.png", pubkey: "bb".repeat(32) },
    { dataUrl: "data:image/png;base64,QUJD", pubkey: "cc".repeat(32) },
  ]);

  assert.deepEqual(
    uploads.map((upload) => upload.pubkey),
    ["cc".repeat(32)],
  );
});

test("an avatar too large to inline can still be published", () => {
  // The whole point of the route: the budget drops this one from the RPC
  // payload, and it still reaches the widget as a published image.
  const big = avatar(CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET + 1_000);
  assert.deepEqual(selectAvatarsWithinBudget([big]), [null]);
  assert.equal(
    toCanvasAvatarUploads([{ dataUrl: big, pubkey: "dd".repeat(32) }]).length,
    1,
  );
});
