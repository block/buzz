import assert from "node:assert/strict";
import test from "node:test";

import { normalizePrivateMobileRelay } from "./privateMobileRelay.ts";

test("normalizes a tailnet hostname to an HTTPS origin", () => {
  assert.deepEqual(
    normalizePrivateMobileRelay("  matthews-macbook-pro-1.tailf29f2c.ts.net  "),
    {
      value: "https://matthews-macbook-pro-1.tailf29f2c.ts.net/",
      error: null,
    },
  );
});

test("normalizes a complete HTTPS tailnet origin", () => {
  assert.deepEqual(
    normalizePrivateMobileRelay(
      "https://matthews-macbook-pro-1.tailf29f2c.ts.net",
    ),
    {
      value: "https://matthews-macbook-pro-1.tailf29f2c.ts.net/",
      error: null,
    },
  );
});

test("treats a blank setting as disabled", () => {
  assert.deepEqual(normalizePrivateMobileRelay("   "), {
    value: "",
    error: null,
  });
});

test("rejects addresses outside the private tailnet origin boundary", () => {
  for (const value of [
    "http://matthews-macbook-pro-1.tailf29f2c.ts.net",
    "https://example.com",
    "https://user@matthews-macbook-pro-1.tailf29f2c.ts.net",
    "https://matthews-macbook-pro-1.tailf29f2c.ts.net/path",
    "https://matthews-macbook-pro-1.tailf29f2c.ts.net?query=1",
    "https://matthews-macbook-pro-1.tailf29f2c.ts.net#fragment",
  ]) {
    const result = normalizePrivateMobileRelay(value);
    assert.equal(result.value, "");
    assert.match(result.error ?? "", /HTTPS.*\.ts\.net origin/);
  }
});
