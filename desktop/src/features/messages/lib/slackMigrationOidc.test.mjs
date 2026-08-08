import assert from "node:assert/strict";
import test from "node:test";

import {
  assertMatchesSlackJoin,
  buildSlackOidcStartUrl,
  createSlackOidcVerifier,
  slackOidcChallenge,
} from "./slackMigrationOidc.ts";

test("S256 challenge matches the RFC 7636 example", async () => {
  assert.equal(
    await slackOidcChallenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
    "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
  );
});

test("creates a high-entropy URL-safe verifier", () => {
  const first = createSlackOidcVerifier();
  const second = createSlackOidcVerifier();
  assert.match(first, /^[A-Za-z0-9_-]{43}$/);
  assert.notEqual(first, second);
});

test("builds a start URL containing only the one-way challenge", async () => {
  const verifier = "v".repeat(43);
  const url = await buildSlackOidcStartUrl(
    "https://migrate.example/",
    verifier,
  );
  assert.match(url, /^https:\/\/migrate\.example\/oidc\/start\?challenge=/);
  assert.ok(!url.includes(verifier));
});

test("callback must match relay, service, and a retained verifier", () => {
  const payload = {
    subject: "slack:T1:U1",
    via: "oidc",
    code: "code",
    relayUrl: "WSS://RELAY.EXAMPLE/",
    service: "https://MIGRATE.EXAMPLE/",
  };
  assert.doesNotThrow(() =>
    assertMatchesSlackJoin(
      payload,
      "wss://relay.example",
      "https://migrate.example",
      "v".repeat(43),
    ),
  );
  assert.throws(() =>
    assertMatchesSlackJoin(
      payload,
      "wss://relay.example",
      "https://migrate.example",
      undefined,
    ),
  );
  assert.throws(() =>
    assertMatchesSlackJoin(
      payload,
      "wss://another.example",
      "https://migrate.example",
      "v".repeat(43),
    ),
  );
});
