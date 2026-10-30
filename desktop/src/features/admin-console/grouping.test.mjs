/**
 * Unit tests for community grouping of deployment-wide admin rows.
 *
 * The admin API returns reports and feedback across every community on the
 * deployment; the console buckets them by community for triage. Grouping must
 * preserve first-seen community order and server row order within a community,
 * and never head a group with an empty host.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { groupByCommunity } from "./AdminConsolePanelHelpers.tsx";

test("group-empty-returns-empty: no rows → no groups", () => {
  assert.deepEqual(groupByCommunity([]), []);
});

test("group-single-community-one-bucket: rows sharing a community collapse to one group", () => {
  const rows = [
    { communityId: "c1", communityHost: "a.example.com", id: "r1" },
    { communityId: "c1", communityHost: "a.example.com", id: "r2" },
  ];
  const groups = groupByCommunity(rows);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].communityId, "c1");
  assert.equal(groups[0].communityHost, "a.example.com");
  assert.deepEqual(
    groups[0].items.map((r) => r.id),
    ["r1", "r2"],
  );
});

test("group-preserves-first-seen-order: communities keep the order they first appear", () => {
  const rows = [
    { communityId: "c2", communityHost: "b.example.com", id: "r1" },
    { communityId: "c1", communityHost: "a.example.com", id: "r2" },
    { communityId: "c2", communityHost: "b.example.com", id: "r3" },
  ];
  const groups = groupByCommunity(rows);
  assert.deepEqual(
    groups.map((g) => g.communityId),
    ["c2", "c1"],
  );
  // Interleaved rows for c2 stay together in server order.
  assert.deepEqual(
    groups[0].items.map((r) => r.id),
    ["r1", "r3"],
  );
});

test("group-blank-host-falls-back-to-id: an empty host never heads a group", () => {
  const rows = [{ communityId: "c1", communityHost: "", id: "r1" }];
  const groups = groupByCommunity(rows);
  assert.equal(groups[0].communityHost, "c1");
});
