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

test("resolveManagedAgentAvatarUrl passes emoji svg data URLs through", async () => {
  const emojiUrl =
    "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%3E%3C%2Fsvg%3E";
  const uploaded = await resolveManagedAgentAvatarUrl(emojiUrl, async () => {
    throw new Error("should not upload inline emoji svg data URLs");
  });

  assert.equal(uploaded, emojiUrl);
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
