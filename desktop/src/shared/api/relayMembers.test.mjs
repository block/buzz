import assert from "node:assert/strict";
import test from "node:test";

import {
  loadRelayMembershipLookup,
  shouldWarnMissingMembershipSnapshot,
} from "./relayMembers.ts";

test("open relays skip the membership snapshot request", async () => {
  const lookup = await loadRelayMembershipLookup("a".repeat(64), false, () =>
    assert.fail("open relays must not request a membership snapshot"),
  );

  assert.deepEqual(lookup, {
    snapshotFound: false,
    membershipRequired: false,
    membership: null,
  });
});

test("membership relays request their membership snapshot", async () => {
  const pubkey = "a".repeat(64);
  let requestCount = 0;
  const lookup = await loadRelayMembershipLookup(pubkey, true, async () => {
    requestCount += 1;
    return {
      created_at: 1,
      tags: [["member", pubkey, "admin"]],
    };
  });

  assert.equal(requestCount, 1);
  assert.equal(lookup.snapshotFound, true);
  assert.equal(lookup.membershipRequired, true);
  assert.equal(lookup.membership?.role, "admin");
});

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
