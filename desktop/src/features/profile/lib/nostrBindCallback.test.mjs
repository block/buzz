import assert from "node:assert/strict";
import test from "node:test";

import { buildNostrBindCallbackUrl } from "./nostrBindCallback.ts";

function decodePayload(callbackUrl) {
  const encoded = new URL(callbackUrl).hash.split("v1.")[1];
  const padded = encoded
    .replaceAll("-", "+")
    .replaceAll("_", "/")
    .padEnd(Math.ceil(encoded.length / 4) * 4, "=");
  return new TextDecoder().decode(
    Uint8Array.from(atob(padded), (character) => character.charCodeAt(0)),
  );
}

test("buildNostrBindCallbackUrl returns a UTF-8 base64url payload in the fragment", () => {
  const response = JSON.stringify({ content: "Buzz ⚡", sig: "+/=" });
  const result = buildNostrBindCallbackUrl(
    "https://example.com/buzz?source=bind",
    response,
  );
  const url = new URL(result);

  assert.equal(url.origin, "https://example.com");
  assert.equal(url.pathname, "/buzz");
  assert.equal(url.search, "?source=bind");
  assert.match(url.hash, /^#buzz_bind=v1\.[A-Za-z0-9_-]+$/);
  assert.equal(decodePayload(result), response);
  assert.equal(url.searchParams.has("buzz_bind"), false);
});

test("buildNostrBindCallbackUrl replaces an existing fragment", () => {
  const result = buildNostrBindCallbackUrl(
    "https://example.com/buzz#stale-fragment",
    "signed",
  );

  assert.equal(new URL(result).hash, "#buzz_bind=v1.c2lnbmVk");
});

test("buildNostrBindCallbackUrl rejects callback URLs beyond the opener ceiling", () => {
  assert.throws(
    () =>
      buildNostrBindCallbackUrl("https://example.com/buzz", "x".repeat(4_096)),
    /too large/,
  );
});
