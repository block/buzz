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
