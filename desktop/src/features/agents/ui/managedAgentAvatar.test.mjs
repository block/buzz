import assert from "node:assert/strict";
import test from "node:test";

import { resolveManagedAgentAvatarUrl } from "./managedAgentAvatar.ts";

test("resolveManagedAgentAvatarUrl uploads data image URIs", async () => {
  const uploaded = await resolveManagedAgentAvatarUrl(
    "data:image/png;base64,aGVsbG8=",
    async (bytes) => {
      assert.deepEqual(bytes, [104, 101, 108, 108, 111]);
      return {
        url: "https://relay.example/avatar.png",
        sha256: "hash",
        size: bytes.length,
        type: "image/png",
        uploaded: 1,
      };
    },
  );

  assert.equal(uploaded, "https://relay.example/avatar.png");
});

test("resolveManagedAgentAvatarUrl squares legacy emoji svg data URLs", async () => {
  const legacySvg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" rx="256" fill="#ffcc00"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258">🐝</text></svg>';
  const emojiUrl = `data:image/svg+xml,${encodeURIComponent(legacySvg)}`;
  const resolved = await resolveManagedAgentAvatarUrl(emojiUrl, async () => {
    throw new Error("should not upload inline emoji svg data URLs");
  });

  assert.ok(resolved);
  const normalizedSvg = decodeURIComponent(resolved.split(",", 2)[1]);
  assert.match(
    normalizedSvg,
    /<rect width="512" height="512" fill="#ffcc00"\/>/u,
  );
  assert.doesNotMatch(normalizedSvg, /\brx=/u);
  assert.match(normalizedSvg, />🐝<\/text>/u);
});

test("resolveManagedAgentAvatarUrl passes non-data URLs through", async () => {
  const uploaded = await resolveManagedAgentAvatarUrl(
    " https://relay.example/already-hosted.png ",
    async () => {
      throw new Error("should not upload hosted avatars");
    },
  );

  assert.equal(uploaded, "https://relay.example/already-hosted.png");
});

test("resolveManagedAgentAvatarUrl omits invalid data image URIs", async () => {
  const uploaded = await resolveManagedAgentAvatarUrl(
    "data:image/png;base64,",
    async () => {
      throw new Error("should not upload invalid data URIs");
    },
  );

  assert.equal(uploaded, undefined);
});

test("resolveManagedAgentAvatarUrl uses safe fallback when data image upload fails", async () => {
  const uploaded = await resolveManagedAgentAvatarUrl(
    "data:image/png;base64,YQ==",
    async () => {
      throw new Error("upload failed");
    },
    "app-avatar://goose",
  );

  assert.equal(uploaded, "app-avatar://goose");
});

test("resolveManagedAgentAvatarUrl ignores data URI fallbacks", async () => {
  const uploaded = await resolveManagedAgentAvatarUrl(
    "data:image/png;base64,YQ==",
    async () => {
      throw new Error("upload failed");
    },
    "data:image/png;base64,Yg==",
  );

  assert.equal(uploaded, undefined);
});
