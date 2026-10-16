import assert from "node:assert/strict";
import test from "node:test";

import { relayClient } from "./relayClient.ts";
import {
  addRelayMember,
  canManageCommunityMembers,
  changeRelayMemberRole,
  removeRelayMember,
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

test("community member management is hidden on open relays", () => {
  assert.equal(
    canManageCommunityMembers({
      snapshotFound: false,
      membershipRequired: false,
      membership: null,
    }),
    false,
  );
});

test("community member management is visible to closed-relay admins and owners", () => {
  for (const role of ["admin", "owner"]) {
    assert.equal(
      canManageCommunityMembers({
        snapshotFound: true,
        membershipRequired: true,
        membership: { pubkey: "a".repeat(64), role },
      }),
      true,
    );
  }
});

test("community member management stays hidden while loading and from closed-relay non-admins", () => {
  assert.equal(canManageCommunityMembers(undefined), false);
  assert.equal(
    canManageCommunityMembers({
      snapshotFound: true,
      membershipRequired: true,
      membership: { pubkey: "a".repeat(64), role: "member" },
    }),
    false,
  );
  assert.equal(
    canManageCommunityMembers({
      snapshotFound: true,
      membershipRequired: true,
      membership: null,
    }),
    false,
  );
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

function installRecordingTauri(t) {
  const invoked = [];
  const previousWindow = globalThis.window;
  globalThis.window = {
    ...previousWindow,
    __TAURI_INTERNALS__: {
      invoke: async (command) => {
        invoked.push(command);
        throw new Error(`unexpected ${command}`);
      },
    },
  };
  t.after(() => {
    relayClient.disconnect();
    globalThis.window = previousWindow;
  });
  return invoked;
}

test("relay admin events refuse a connection to another community's relay", async (t) => {
  const invoked = installRecordingTauri(t);
  relayClient.bindRelayUrl("wss://self-hosted.example");
  const target = "b".repeat(64);
  const selected = "wss://hosted.example";

  for (const publish of [
    () => addRelayMember(target, "admin", selected),
    () => removeRelayMember(target, selected),
    () => changeRelayMemberRole(target, "member", selected),
    () => addRelayMember(target, "admin", undefined),
  ]) {
    await assert.rejects(publish, /is not the selected community/);
  }
  assert.deepEqual(invoked, [], "nothing may be signed or sent");
});

test("relay admin events for the connected community proceed to signing", async (t) => {
  const invoked = installRecordingTauri(t);
  relayClient.bindRelayUrl("wss://hosted.example");

  await assert.rejects(
    () => addRelayMember("b".repeat(64), "admin", "wss://hosted.example"),
    /unexpected sign_event/,
  );
  assert.deepEqual(invoked, ["sign_event"]);
});
