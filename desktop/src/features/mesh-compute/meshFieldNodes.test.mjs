import assert from "node:assert/strict";
import test from "node:test";

/**
 * Contract tests for the radar field's node source.
 *
 * The load-bearing claim: the community's shared compute is visible **without**
 * running a node, because `mesh_snapshot` reads other members' relay status
 * notes. An earlier version gated the graph on a local runtime and told the user
 * to "turn on sharing to see who's on the mesh", which was false.
 *
 * The equally load-bearing counter-claim: relay notes are *last known*, not
 * live. These tests pin that the two sources stay distinguishable — no RTT and
 * no claimed membership from notes.
 */

import { deriveMeshFieldModel } from "./meshFieldNodes.ts";

function device(overrides = {}) {
  return {
    deviceId: "d1",
    label: "studio54",
    capacityGb: 36,
    models: ["gemma"],
    state: "serving",
    isSelf: false,
    ...overrides,
  };
}

function snapshot(overrides = {}) {
  return {
    sharingDeviceCount: 1,
    sharedCapacityGb: 36,
    models: [],
    devices: [device()],
    includesSelf: false,
    memberCount: 1,
    reason: null,
    ...overrides,
  };
}

function peer(overrides = {}) {
  return {
    id: "p1",
    label: "studio54",
    state: "serving",
    capacityGb: 36,
    models: [],
    rttMs: 30,
    ...overrides,
  };
}

test("a live runtime wins: only it can vouch for reachability", () => {
  const field = deriveMeshFieldModel({
    view: { connected: true, selfCapacityGb: 115, peers: [peer()] },
    snapshot: snapshot(),
  });
  assert.equal(field.source, "live");
  assert.equal(field.selfParticipating, true);
  assert.equal(field.selfCapacityGb, 115);
  assert.equal(field.nodes[0].rttMs, 30);
});

test("with no runtime, relay notes still show the pool", () => {
  // The whole point: you can see there is a mesh before joining it.
  const field = deriveMeshFieldModel({
    view: null,
    snapshot: snapshot({
      sharingDeviceCount: 2,
      devices: [device(), device({ deviceId: "d2" })],
    }),
  });
  assert.equal(field.source, "relay");
  assert.equal(field.nodes.length, 2);
});

test("relay nodes carry no RTT and claim no membership", () => {
  const field = deriveMeshFieldModel({ view: null, snapshot: snapshot() });
  // Inventing an RTT would place a node at a distance that means nothing.
  assert.equal(field.nodes[0].rttMs, null);
  // A hollow centre and no spokes depend on this staying false.
  assert.equal(field.selfParticipating, false);
  assert.equal(field.selfCapacityGb, null);
});

test("the relay view is not hedged with a caption", () => {
  // What we know is shared gets shown as shared. Annotating it with "last
  // reported" hedged a fact we actually have, and the old "join to see live
  // detail" mislabeled sharing as joining.
  const field = deriveMeshFieldModel({ view: null, snapshot: snapshot() });
  assert.equal(field.caption, null);
});

test("our own note is excluded so we are never drawn twice", () => {
  const field = deriveMeshFieldModel({
    view: null,
    snapshot: snapshot({
      devices: [device(), device({ deviceId: "self", isSelf: true })],
    }),
  });
  assert.equal(field.nodes.length, 1);
});

test("a device id is optional; ring slot keeps keys stable", () => {
  const field = deriveMeshFieldModel({
    view: null,
    snapshot: snapshot({
      devices: [device({ deviceId: null }), device({ deviceId: null })],
    }),
  });
  assert.notEqual(field.nodes[0].id, field.nodes[1].id);
});

test("ghosts are a count, never capacity", () => {
  const field = deriveMeshFieldModel({
    view: null,
    snapshot: snapshot({ sharingDeviceCount: 1, memberCount: 12 }),
  });
  assert.equal(field.ghostCount, 11);
  // No ghost contributes to a capacity figure anywhere in this model.
  assert.equal(field.selfCapacityGb, null);
});

test("an empty community is 'none', not an empty live view", () => {
  const field = deriveMeshFieldModel({
    view: null,
    snapshot: snapshot({ sharingDeviceCount: 0, devices: [], memberCount: 0 }),
  });
  assert.equal(field.source, "none");
  assert.equal(field.caption, null);
});

test("a solo sharer is told it is waiting, not that it is alone in error", () => {
  const field = deriveMeshFieldModel({
    view: { connected: true, selfCapacityGb: 115, peers: [] },
    snapshot: null,
  });
  assert.equal(field.source, "live");
  assert.equal(field.caption, "Waiting for another device");
});
