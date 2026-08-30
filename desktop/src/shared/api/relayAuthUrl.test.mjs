import assert from "node:assert/strict";
import test from "node:test";

import { relayAuthUrlForTransport } from "./relayAuthUrl.ts";

test("LAN auth uses ws while preserving the canonical community host", () => {
  assert.equal(
    relayAuthUrlForTransport(
      "wss://content-swift-seemingly.ngrok-free.app",
      "lan",
    ),
    "ws://content-swift-seemingly.ngrok-free.app",
  );
});

test("LAN auth preserves an already-plain canonical relay URL", () => {
  assert.equal(
    relayAuthUrlForTransport("ws://relay.example:3000", "lan"),
    "ws://relay.example:3000",
  );
});

test("public and not-yet-reported transports preserve canonical TLS", () => {
  const canonical = "wss://relay.example";
  assert.equal(relayAuthUrlForTransport(canonical, "public"), canonical);
  assert.equal(relayAuthUrlForTransport(canonical, null), canonical);
});
