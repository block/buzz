import assert from "node:assert/strict";
import test from "node:test";

import {
  isAgentSnapshotPngFilename,
  withAgentSnapshotAvatarPreview,
} from "./agentSnapshotAttachmentPreview.ts";

const descriptor = {
  filename: "honey.agent.png",
  sha256: "a".repeat(64),
  size: 1234,
  type: "image/png",
  uploaded: 1,
  url: `https://relay.example/media/${"a".repeat(64)}.png`,
};

test("recognizes only agent snapshot PNG filenames", () => {
  assert.equal(isAgentSnapshotPngFilename("Honey.AGENT.PNG"), true);
  assert.equal(isAgentSnapshotPngFilename("honey.agent.json"), false);
  assert.equal(isAgentSnapshotPngFilename("honey.png"), false);
});

test("recovers the source avatar from local exported snapshot bytes", async () => {
  let fetchCount = 0;
  const enriched = await withAgentSnapshotAvatarPreview(
    descriptor,
    new Uint8Array([1, 2, 3]),
    {
      fetchBytes: async () => {
        fetchCount += 1;
        return [];
      },
      previewImport: async (bytes, filename) => {
        assert.deepEqual(bytes, [1, 2, 3]);
        assert.equal(filename, "honey.agent.png");
        return {
          avatarUrl: "data:image/png;base64,portable-avatar",
          sourceAvatarUrl: "https://relay.example/media/honey-avatar.png",
        };
      },
    },
  );

  assert.equal(fetchCount, 0);
  assert.equal(
    enriched.snapshotAvatarUrl,
    "https://relay.example/media/honey-avatar.png",
  );
  assert.equal(enriched.thumb, undefined);
});

test("fetches an already-uploaded snapshot for native picker attachments", async () => {
  let fetchedArgs;
  const enriched = await withAgentSnapshotAvatarPreview(descriptor, undefined, {
    fetchBytes: async (args) => {
      fetchedArgs = args;
      return [4, 5, 6];
    },
    previewImport: async () => ({
      avatarUrl: "data:image/png;base64,avatar",
      sourceAvatarUrl: null,
    }),
  });

  assert.deepEqual(fetchedArgs, {
    url: descriptor.url,
    filename: descriptor.filename,
    expectedSha256: descriptor.sha256,
    expectedSize: descriptor.size,
  });
  assert.equal(enriched.snapshotAvatarUrl, "data:image/png;base64,avatar");
});

test("preview failures keep the full-card fallback", async () => {
  const result = await withAgentSnapshotAvatarPreview(descriptor, [1], {
    previewImport: async () => {
      throw new Error("malformed snapshot");
    },
  });

  assert.equal(result, descriptor);
});
