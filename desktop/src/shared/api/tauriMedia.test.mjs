import assert from "node:assert/strict";
import test from "node:test";

import { clipboardArrayBufferToBytes } from "./tauriMedia.ts";

test("clipboardArrayBufferToBytes preserves raw PNG IPC bytes", () => {
  const png = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
  assert.deepEqual([...clipboardArrayBufferToBytes(png.buffer)], [...png]);
});
