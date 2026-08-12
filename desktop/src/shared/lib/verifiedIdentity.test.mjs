import assert from "node:assert/strict";
import test from "node:test";

import {
  getCurrentVerifiedName,
  millisecondsUntilVerifiedIdentityExpiry,
  withCurrentVerifiedIdentity,
} from "./verifiedIdentity.ts";

const NOW_MS = 1_800_000_000_000;
const NOW_SECONDS = NOW_MS / 1_000;

test("returns a verified name only before its local expiration", () => {
  assert.equal(
    getCurrentVerifiedName(" Example ", NOW_SECONDS + 60, NOW_MS),
    "Example",
  );
  assert.equal(getCurrentVerifiedName("Example", NOW_SECONDS, NOW_MS), null);
});

test("missing and malformed expirations fail closed", () => {
  assert.equal(getCurrentVerifiedName("Example", null, NOW_MS), null);
  assert.equal(
    getCurrentVerifiedName("Example", NOW_SECONDS + 0.5, NOW_MS),
    null,
  );
  assert.equal(getCurrentVerifiedName("Example", Number.NaN, NOW_MS), null);
});

test("computes the exact cutoff delay used by expiry render timers", () => {
  assert.equal(
    millisecondsUntilVerifiedIdentityExpiry(NOW_SECONDS + 60, NOW_MS),
    60_000,
  );
  assert.equal(
    millisecondsUntilVerifiedIdentityExpiry(NOW_SECONDS - 1, NOW_MS),
    0,
  );
});

test("sanitizes cached identity objects without churning valid values", () => {
  const valid = {
    verifiedName: "Example",
    verifiedNameExpiresAt: NOW_SECONDS + 60,
  };
  assert.equal(withCurrentVerifiedIdentity(valid, NOW_MS), valid);
  assert.deepEqual(withCurrentVerifiedIdentity(valid, NOW_MS + 60_000), {
    verifiedName: null,
    verifiedNameExpiresAt: NOW_SECONDS + 60,
  });
});
