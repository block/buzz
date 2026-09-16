import assert from "node:assert/strict";
import { test } from "node:test";
import {
  bindNativeIdentity,
  revokeNativeIdentity,
} from "./nativeIdentitySession.ts";
import { uploadMediaFile } from "./tauriMedia.ts";

test("raw file IPC retains its original generation across a held browser read", async () => {
  const previous = globalThis.window;
  const calls = [];
  globalThis.window = {
    btoa,
    __TAURI_INTERNALS__: {
      invoke: async (...args) => {
        calls.push(args);
        throw "stale native generation";
      },
    },
  };
  try {
    bindNativeIdentity({
      mode: "remote",
      authState: "authenticated",
      publicIdentity: "owner",
      generation: 7,
      workspaceActive: true,
    });
    let complete;
    let entered = false;
    const file = {
      name: "notes 🐝.txt",
      arrayBuffer() {
        entered = true;
        return new Promise((resolve) => {
          complete = resolve;
        });
      },
    };
    const pending = uploadMediaFile(file, "progress-id");
    assert.equal(entered, true);
    revokeNativeIdentity();
    complete(new Uint8Array([0, 255, 42]).buffer);
    await assert.rejects(pending, /stale native generation/);
    assert.equal(calls.length, 1);
    const [command, bytes, options] = calls[0];
    assert.equal(command, "upload_media_bytes_raw");
    assert.ok(bytes instanceof Uint8Array);
    assert.deepEqual([...bytes], [0, 255, 42]);
    assert.equal(options.headers["x-buzz-identity-generation"], "7");
    assert.equal(
      Buffer.from(options.headers["x-buzz-filename"], "base64url").toString(),
      file.name,
    );
    assert.equal(
      Buffer.from(
        options.headers["x-buzz-progress-id"],
        "base64url",
      ).toString(),
      "progress-id",
    );
    await assert.rejects(uploadMediaFile(file), /Sign in/);
    assert.equal(calls.length, 1);
  } finally {
    globalThis.window = previous;
  }
});
