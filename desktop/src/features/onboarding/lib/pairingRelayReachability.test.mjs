import assert from "node:assert/strict";
import test from "node:test";

const {
  classifyPairingQrUri,
  classifyPairingRelay,
  localOnlyPairingRelayMessage,
  pairingRelayFromQrUri,
} = await import("./pairingRelayReachability.ts");

// The code a fresh desktop emits when no community relay is configured: the
// built-in dev default from src-tauri/src/relay.rs, percent-encoded by the
// Rust side.
const DEFAULT_RELAY_RECOVERY_URI =
  "nostrpair://bb02eb1cf24d3590b344a514e2477dc335fd018f799d99161a6a965078a058e8" +
  "?secret=cdb535bf5d3402a4c178bc395d5200c7d230e1ad77f218afa020346be31da380" +
  "&relay=ws%3A%2F%2Flocalhost%3A3000&v=1&mode=recover";

// Same shape once the desktop is pointed at a real community: the relay's
// NIP-11 document advertises a public pairing relay. Dots arrive as `%2E`.
const PUBLIC_RELAY_RECOVERY_URI =
  "nostrpair://59e937d51a4755be8255813f867d32ef3219cd7563999ca5905fa707e5ecacd2" +
  "?secret=602d497bb8c71d483694ff386c28dd9601621c4b447f667a42b5b96228afb4ff" +
  "&relay=wss%3A%2F%2Fpairing%2Ebuzz%2Exyz&v=1&mode=recover";

test("extracts and decodes the relay the code asks the phone to join", () => {
  assert.equal(
    pairingRelayFromQrUri(DEFAULT_RELAY_RECOVERY_URI),
    "ws://localhost:3000",
  );
  assert.equal(
    pairingRelayFromQrUri(PUBLIC_RELAY_RECOVERY_URI),
    "wss://pairing.buzz.xyz",
  );
  assert.equal(pairingRelayFromQrUri("nostrpair://abc?secret=1&v=1"), null);
  assert.equal(pairingRelayFromQrUri("not a uri"), null);
});

test("the unconfigured default relay is flagged as local-only", () => {
  const result = classifyPairingQrUri(DEFAULT_RELAY_RECOVERY_URI);
  assert.equal(result.kind, "local-only");
  assert.equal(result.host, "localhost");
  assert.equal(result.reason, "loopback");
  assert.equal(result.relayUrl, "ws://localhost:3000");
});

test("a public pairing relay is reachable", () => {
  const result = classifyPairingQrUri(PUBLIC_RELAY_RECOVERY_URI);
  assert.equal(result.kind, "reachable");
  assert.equal(result.host, "pairing.buzz.xyz");
});

test("loopback, unspecified and *.localhost hosts are local-only", () => {
  for (const url of [
    "ws://127.0.0.1:3000",
    "ws://[::1]:3000",
    "ws://0.0.0.0:3000",
    "ws://relay.localhost",
    "wss://LOCALHOST",
  ]) {
    const result = classifyPairingRelay(url);
    assert.equal(result.kind, "local-only", url);
  }
});

test("RFC 1918 and link-local IPv4 ranges are local-only — the phone rejects them too", () => {
  for (const url of [
    "ws://10.0.0.5:3000",
    "ws://172.16.0.1:3000",
    "ws://172.31.255.254:3000",
    "ws://192.168.1.20:3000",
    "ws://169.254.10.10:3000",
  ]) {
    const result = classifyPairingRelay(url);
    assert.equal(result.kind, "local-only", url);
    assert.equal(result.reason, "private-network", url);
  }
});

test("public IPv4 and hostnames are reachable; 172.x outside /12 is public", () => {
  for (const url of [
    "wss://pairing.buzz.xyz",
    "wss://nas-dum.communities.buzz.xyz",
    "ws://8.8.8.8:3000",
    "ws://172.15.0.1:3000",
    "ws://172.32.0.1:3000",
    "ws://11.0.0.1",
  ]) {
    assert.equal(classifyPairingRelay(url).kind, "reachable", url);
  }
});

test("garbage input is unknown rather than a false positive either way", () => {
  assert.equal(classifyPairingRelay("").kind, "unknown");
  assert.equal(classifyPairingRelay("::::").kind, "unknown");
  assert.equal(classifyPairingQrUri("nostrpair://abc?v=1").kind, "unknown");
});

test("the local-only message names the offending address and the fix", () => {
  const result = classifyPairingRelay("ws://localhost:3000");
  assert.equal(result.kind, "local-only");
  const message = localOnlyPairingRelayMessage(result);
  assert.match(message, /ws:\/\/localhost:3000/);
  assert.match(message, /this computer only/);
  assert.match(message, /invite link or relay address/);

  const lan = classifyPairingRelay("ws://192.168.1.20:3000");
  assert.equal(lan.kind, "local-only");
  assert.match(localOnlyPairingRelayMessage(lan), /private network address/);
});
