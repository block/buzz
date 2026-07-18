import assert from "node:assert/strict";
import test from "node:test";

import { shouldWarnMissingMembershipSnapshot } from "./relayMembers.ts";

test("missing snapshot warns when the relay requires membership", () => {
  assert.equal(
    shouldWarnMissingMembershipSnapshot({
      snapshotFound: false,
      membershipRequired: true,
      membership: null,
    }),
    true,
  );
});

test("missing snapshot is normal on an open relay", () => {
  assert.equal(
    shouldWarnMissingMembershipSnapshot({
      snapshotFound: false,
      membershipRequired: false,
      membership: null,
    }),
    false,
  );
});

test("an available snapshot never warns", () => {
  assert.equal(
    shouldWarnMissingMembershipSnapshot({
      snapshotFound: true,
      membershipRequired: true,
      membership: null,
    }),
    false,
  );
});
