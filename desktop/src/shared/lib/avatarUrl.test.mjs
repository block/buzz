import assert from "node:assert/strict";
import test from "node:test";

import { isRelayHostedAvatarUrl, resolveAvatarImageSrc } from "./avatarUrl.ts";

const HASH = "a".repeat(64);
const RELAY_ORIGIN = "https://relay.example";

test("isRelayHostedAvatarUrl accepts image media on the active relay", () => {
  assert.equal(
    isRelayHostedAvatarUrl(`${RELAY_ORIGIN}/media/${HASH}.png`, RELAY_ORIGIN),
    true,
  );
  assert.equal(
    isRelayHostedAvatarUrl(`${RELAY_ORIGIN}/media/${HASH}`, RELAY_ORIGIN),
    true,
  );
  assert.equal(
    isRelayHostedAvatarUrl(
      `${RELAY_ORIGIN}/media/${HASH}.thumb.jpg`,
      RELAY_ORIGIN,
    ),
    true,
  );
});

test("isRelayHostedAvatarUrl rejects external media and non-image media", () => {
  assert.equal(
    isRelayHostedAvatarUrl(
      `https://nostr.build/media/${HASH}.png`,
      RELAY_ORIGIN,
    ),
    false,
  );
  assert.equal(
    isRelayHostedAvatarUrl(`${RELAY_ORIGIN}/media/${HASH}.mp4`, RELAY_ORIGIN),
    false,
  );
  assert.equal(
    isRelayHostedAvatarUrl(`${RELAY_ORIGIN}/other/${HASH}.png`, RELAY_ORIGIN),
    false,
  );
  assert.equal(
    isRelayHostedAvatarUrl(
      `${RELAY_ORIGIN}/media/${HASH}.thumb.png`,
      RELAY_ORIGIN,
    ),
    false,
  );
});

test("resolveAvatarImageSrc fails closed for unresolved relay origin", () => {
  assert.equal(
    resolveAvatarImageSrc(`${RELAY_ORIGIN}/media/${HASH}.png`, null),
    null,
  );
});

test("resolveAvatarImageSrc preserves inline previews and blocks external URLs", () => {
  assert.equal(
    resolveAvatarImageSrc("blob:local-preview", null),
    "blob:local-preview",
  );
  assert.equal(
    resolveAvatarImageSrc("data:image/svg+xml,%3Csvg%3E%3C/svg%3E", null),
    "data:image/svg+xml,%3Csvg%3E%3C/svg%3E",
  );
  assert.equal(
    resolveAvatarImageSrc(
      `https://nostr.build/media/${HASH}.png`,
      RELAY_ORIGIN,
    ),
    null,
  );
});
