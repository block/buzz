import assert from "node:assert/strict";
import test from "node:test";

import { importAvatarUrl, parseImportUrl } from "./avatarUrlImport.ts";

const HASH = "b".repeat(64);

function response(body, headers = {}) {
  return new Response(body, { headers, status: 200 });
}

test("parseImportUrl rejects non-http URL schemes", () => {
  assert.throws(
    () => parseImportUrl("file:///tmp/avatar.png"),
    /HTTP\(S\) image URL/,
  );
  assert.throws(
    () => parseImportUrl("data:image/png,abc"),
    /HTTP\(S\) image URL/,
  );
});

test("importAvatarUrl fetches client-side bytes and uploads the artefact", async () => {
  const fetched = [];
  const uploaded = [];
  const descriptor = await importAvatarUrl("https://cdn.example/avatar.png", {
    relayOrigin: "https://relay.example",
    fetchFn: async (url) => {
      fetched.push(String(url));
      return response(
        new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" }),
        {
          "content-type": "image/png",
          "content-length": "3",
        },
      );
    },
    uploadFn: async (data, filename) => {
      uploaded.push({ data, filename });
      return {
        sha256: "hash",
        size: data.length,
        type: "image/png",
        uploaded: Date.now(),
        url: `https://relay.example/media/${HASH}.png`,
      };
    },
  });

  assert.deepEqual(fetched, ["https://cdn.example/avatar.png"]);
  assert.deepEqual(uploaded, [{ data: [1, 2, 3], filename: "avatar.png" }]);
  assert.equal(descriptor.url, `https://relay.example/media/${HASH}.png`);
});

test("importAvatarUrl rejects non-image responses before upload", async () => {
  let uploaded = false;
  await assert.rejects(
    () =>
      importAvatarUrl("https://cdn.example/not-image.txt", {
        relayOrigin: "https://relay.example",
        fetchFn: async () =>
          response("hello", {
            "content-type": "text/plain",
            "content-length": "5",
          }),
        uploadFn: async () => {
          uploaded = true;
          throw new Error("unreachable");
        },
      }),
    /PNG, JPG, GIF, or WebP/,
  );
  assert.equal(uploaded, false);
});

test("importAvatarUrl refuses oversized images", async () => {
  await assert.rejects(
    () =>
      importAvatarUrl("https://cdn.example/avatar.png", {
        relayOrigin: "https://relay.example",
        fetchFn: async () =>
          response(new Blob(["tiny"], { type: "image/png" }), {
            "content-type": "image/png",
            "content-length": String(11 * 1024 * 1024),
          }),
        uploadFn: async () => {
          throw new Error("unreachable");
        },
      }),
    /too large/,
  );
});
