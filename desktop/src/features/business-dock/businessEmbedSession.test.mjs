import assert from "node:assert/strict";
import test from "node:test";

import {
  businessEmbedCallbackUrl,
  buildBusinessEmbedBootstrapUrl,
  buildBusinessEmbedLoginUrl,
  canAttemptBusinessRecovery,
  parseBusinessEmbedCallback,
} from "./businessEmbedSession.ts";

const config = {
  homeUrl: "https://business.bizfin.localhost/embed/",
  origin: "https://business.bizfin.localhost",
};

test("embed login binds the request to an allowed Business target", () => {
  const value = buildBusinessEmbedLoginUrl(
    config,
    "https://business.bizfin.localhost/embed/orders/42?tab=lines",
  );
  const url = new URL(value);
  assert.equal(url.pathname, "/auth/embed-login");
  assert.equal(url.searchParams.get("target"), "/embed/orders/42?tab=lines");
  assert.equal(
    buildBusinessEmbedLoginUrl(config, "https://evil.example/embed/orders/42"),
    null,
  );
});

test("desktop callback accepts only the exact scheme and 256-bit code", () => {
  const code = "a".repeat(43);
  assert.equal(
    parseBusinessEmbedCallback(
      `buzz://auth/business-bootstrap?code=${code}`,
      businessEmbedCallbackUrl("production"),
    ),
    code,
  );
  assert.equal(
    parseBusinessEmbedCallback(
      `buzz-dev://auth/business-bootstrap?code=${code}`,
      businessEmbedCallbackUrl("development"),
    ),
    code,
  );
  assert.equal(
    parseBusinessEmbedCallback(
      `buzz://auth/business-bootstrap?code=${code}`,
      businessEmbedCallbackUrl("development"),
    ),
    null,
  );
  for (const value of [
    `buzz://evil/business-bootstrap?code=${code}`,
    `buzz://auth/business-bootstrap?code=${code}&token=leak`,
    "buzz://auth/business-bootstrap?code=short",
    `https://auth/business-bootstrap?code=${code}`,
  ])
    assert.equal(parseBusinessEmbedCallback(value), null);
});

test("bootstrap URL stays on the configured Business origin", () => {
  const code = "b".repeat(43);
  assert.equal(
    buildBusinessEmbedBootstrapUrl(config, code),
    `https://business.bizfin.localhost/embed/bootstrap?code=${code}`,
  );
  assert.equal(buildBusinessEmbedBootstrapUrl(config, "bad"), null);
});

test("automatic recovery is capped at one attempt", () => {
  assert.equal(canAttemptBusinessRecovery(0), true);
  assert.equal(canAttemptBusinessRecovery(1), false);
  assert.equal(canAttemptBusinessRecovery(2), false);
});
