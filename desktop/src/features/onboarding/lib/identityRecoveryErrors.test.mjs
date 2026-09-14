import assert from "node:assert/strict";
import test from "node:test";

const { recoveryErrorMessage } = await import("./identityRecoveryErrors.ts");

const EXPIRED =
  "This pairing code expired or lost its connection. Create a new code and try again.";

test("connection failures name the pairing relay when it is known", () => {
  for (const backendMessage of [
    "relay connection closed",
    "WebSocket handshake failed: Connection refused (os error 111)",
    "failed to connect to relay",
  ]) {
    const message = recoveryErrorMessage(backendMessage, "ws://localhost:3000");
    assert.match(message, /ws:\/\/localhost:3000/, backendMessage);
    assert.match(message, /community address/, backendMessage);
    assert.doesNotMatch(message, /expired/, backendMessage);
  }
});

test("connection failures without a known relay keep the expired wording", () => {
  assert.equal(recoveryErrorMessage("relay connection closed"), EXPIRED);
  assert.equal(recoveryErrorMessage("websocket error", null), EXPIRED);
});

test("expiry, timeouts and SAS-confirm failures are unchanged, relay or not", () => {
  for (const backendMessage of [
    "pairing session expired",
    "sas-confirm rejected by peer",
    "request timed out",
  ]) {
    assert.equal(recoveryErrorMessage(backendMessage), EXPIRED);
    assert.equal(
      recoveryErrorMessage(backendMessage, "wss://pairing.buzz.xyz"),
      EXPIRED,
    );
  }
});

test("unrecognised backend messages pass through verbatim", () => {
  assert.equal(
    recoveryErrorMessage("invalid relay URL: empty host", "ws://x"),
    "invalid relay URL: empty host",
  );
});
