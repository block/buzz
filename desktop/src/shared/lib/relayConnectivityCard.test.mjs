import assert from "node:assert/strict";
import test from "node:test";

import {
  isBlockRelayUrl,
  resolveRelayConnectivityCardVariant,
} from "./relayConnectivityCard.ts";

test("isBlockRelayUrl recognizes Block-owned relay hosts", () => {
  assert.equal(isBlockRelayUrl("wss://sprout-oss.stage.blox.sqprod.co"), true);
  assert.equal(isBlockRelayUrl("wss://relay.block.xyz"), true);
  assert.equal(isBlockRelayUrl("wss://relay.squareup.com"), true);
});

test("isBlockRelayUrl rejects custom and malformed relay URLs", () => {
  assert.equal(isBlockRelayUrl("wss://relay.example.com"), false);
  assert.equal(isBlockRelayUrl("not a url"), false);
  assert.equal(isBlockRelayUrl(null), false);
});

test("resolveRelayConnectivityCardVariant keeps custom workspaces generic", () => {
  assert.equal(
    resolveRelayConnectivityCardVariant(
      "relay unreachable: relay returned an unexpected HTML page (VPN or proxy sign-in?)",
      "wss://relay.example.com",
    ),
    "reconnect-relay",
  );
});

test("resolveRelayConnectivityCardVariant offers VPN for generic Block relay failures", () => {
  assert.equal(
    resolveRelayConnectivityCardVariant(
      "relay unreachable: could not connect to relay",
      "wss://sprout-oss.stage.blox.sqprod.co",
    ),
    "connect-vpn",
  );
});

test("resolveRelayConnectivityCardVariant offers VPN for generic Block proxy failures", () => {
  assert.equal(
    resolveRelayConnectivityCardVariant(
      "relay unreachable: relay returned an unexpected HTML page (VPN or proxy sign-in?)",
      "wss://sprout-oss.stage.blox.sqprod.co",
    ),
    "connect-vpn",
  );
});

test("resolveRelayConnectivityCardVariant offers VPN for Block HTTP redirects", () => {
  assert.equal(
    resolveRelayConnectivityCardVariant(
      "HTTP error: 302 Found",
      "wss://sprout-oss.stage.blox.sqprod.co",
    ),
    "connect-vpn",
  );
});

test("resolveRelayConnectivityCardVariant offers VPN for Cloudflare Access redirects without reauth detail", () => {
  assert.equal(
    resolveRelayConnectivityCardVariant(
      "HTTP error: 302 Found - Cloudflare Access sign-in required",
      "wss://sprout-oss.stage.blox.sqprod.co",
    ),
    "connect-vpn",
  );
});

test("resolveRelayConnectivityCardVariant offers access refresh for Cloudflare Access failures", () => {
  assert.equal(
    resolveRelayConnectivityCardVariant(
      "relay unreachable: network sign-in required (Cloudflare Access / VPN) - re-authenticate and reconnect",
      "wss://sprout-oss.stage.blox.sqprod.co",
    ),
    "refresh-access",
  );
});
