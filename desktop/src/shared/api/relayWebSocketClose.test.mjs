import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { closeWebSocket } from "./relayWebSocketClose.ts";

test("closeWebSocket sends a Close frame through plugin:websocket|send", async () => {
  const calls = [];
  await closeWebSocket(42, "community switch", async (cmd, args) => {
    calls.push({ cmd, args });
  });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].cmd, "plugin:websocket|send");
  assert.deepEqual(calls[0].args, {
    id: 42,
    message: {
      type: "Close",
      data: { code: 1000, reason: "community switch" },
    },
  });
});

test("closeWebSocket swallows send failures (socket already gone)", async () => {
  await closeWebSocket(7, "connection reset", async () => {
    throw new Error("WebSocket connection not found");
  });
});

// Regression guard: tauri-plugin-websocket registers only `connect` and
// `send` — there is no `disconnect` command. Invoking one rejects silently
// and leaks the socket (relay zombie pile, community-switch disconnects).
// Any socket teardown must go through closeWebSocket.
test("no source file invokes the nonexistent plugin:websocket|disconnect command", () => {
  const srcRoot = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "..",
    "..",
  );
  const offenders = [];

  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      if (!/\.(ts|tsx|js|jsx|mjs)$/.test(entry.name)) continue;
      if (full === fileURLToPath(import.meta.url)) continue;
      if (
        fs.readFileSync(full, "utf8").includes("plugin:websocket|disconnect")
      ) {
        offenders.push(path.relative(srcRoot, full));
      }
    }
  };
  walk(srcRoot);

  assert.deepEqual(
    offenders,
    [],
    "plugin:websocket|disconnect does not exist in tauri-plugin-websocket — use closeWebSocket (Close frame via plugin:websocket|send) instead",
  );
});
