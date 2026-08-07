import assert from "node:assert/strict";
import test from "node:test";

/**
 * Contract tests for the mesh detail popover projection.
 *
 * The load-bearing rule: mesh-llm exposes **no inbound counter**. Every figure
 * available describes work this machine dispatched, so nothing here may claim
 * another member consumed our compute. Several tests exist purely to pin that.
 */

import {
  describeParticipationHint,
  describeRequestOrigin,
  inferInboundWork,
} from "./meshActivity.ts";
import { describeActivity, deriveMeshDetailModel } from "./meshDetailModel.ts";

function usage(overrides = {}) {
  return {
    inflight: 0,
    peakInflight: 0,
    requestsServed: 0,
    tokensServed: 0,
    tokensPerSecond: 0,
    localAttempts: 0,
    remoteAttempts: 0,
    endpointAttempts: 0,
    peers: 0,
    locallyServed: 0,
    remotelyServed: 0,
    endpointServed: 0,
    ...overrides,
  };
}

function device(overrides = {}) {
  return {
    deviceId: "d1",
    label: "Device",
    capacityGb: 16,
    models: ["m"],
    state: "serving",
    isSelf: false,
    ...overrides,
  };
}

function peer(overrides = {}) {
  return {
    id: "peer-1",
    label: "studio54.lan",
    state: "serving",
    capacityGb: 64,
    models: ["m"],
    rttMs: 30,
    ...overrides,
  };
}

function view(overrides = {}) {
  return {
    connected: true,
    selfCapacityGb: 115,
    peers: [],
    ...overrides,
  };
}

function snapshot(overrides = {}) {
  return {
    sharingDeviceCount: 1,
    sharedCapacityGb: 16,
    models: ["m"],
    devices: [device({ isSelf: true })],
    includesSelf: true,
    memberCount: 1,
    reason: null,
    ...overrides,
  };
}

test("request origin distinguishes borrowed compute from local work", () => {
  assert.equal(
    describeRequestOrigin(usage({ locallyServed: 5 })),
    "5 requests ran here",
  );
  assert.equal(
    describeRequestOrigin(usage({ remotelyServed: 4 })),
    "4 requests ran on shared compute",
  );
  assert.equal(
    describeRequestOrigin(usage({ locallyServed: 9, remotelyServed: 3 })),
    "3 of 12 requests ran on shared compute",
  );
});

test("endpoint-served requests count as shared, not local", () => {
  // An endpoint is not this machine's GPU, so it must not read as local work.
  assert.equal(
    describeRequestOrigin(usage({ locallyServed: 1, endpointServed: 1 })),
    "1 of 2 requests ran on shared compute",
  );
});

test("no completed requests yields no origin claim, never '0 remote'", () => {
  assert.equal(describeRequestOrigin(usage()), null);
  assert.equal(describeRequestOrigin(null), null);
});

test("singular grammar for a single request", () => {
  assert.equal(
    describeRequestOrigin(usage({ locallyServed: 1 })),
    "1 request ran here",
  );
});

test("activity reports inflight work without attributing it to anyone", () => {
  const live = describeActivity({
    usage: usage({ inflight: 2 }),
    isSharing: true,
    inboundWork: false,
  });
  assert.equal(live, "2 requests in flight");
  // Without the elimination check the work may be our own agent's, so the
  // phrasing must never imply someone else is using this machine.
  assert.ok(
    !/(served|for (others|members|someone)|using your|consumed|another member)/i.test(
      live,
    ),
    `must not attribute inbound work: ${live}`,
  );
});

test("inferred inbound work is the one case a peer may be credited", () => {
  assert.equal(
    describeActivity({
      usage: usage({ inflight: 2 }),
      isSharing: true,
      inboundWork: true,
    }),
    "2 requests in flight · from another member",
  );
});

test("a warm sharing runtime with no traffic is its own state", () => {
  assert.equal(
    describeActivity({ usage: usage(), isSharing: true, inboundWork: false }),
    "Ready · no requests yet",
  );
  // Not sharing and idle is not noteworthy — stay silent.
  assert.equal(
    describeActivity({ usage: usage(), isSharing: false, inboundWork: false }),
    null,
  );
  assert.equal(
    describeActivity({ usage: null, isSharing: true, inboundWork: false }),
    null,
  );
});

test("inbound work requires two samples and a flat dispatch count", () => {
  const base = { isSharing: true };
  // Our own dispatch count moved, so at least some of the work is ours.
  assert.equal(
    inferInboundWork({
      ...base,
      current: { inflight: 1, requestsRouted: 6 },
      previous: { inflight: 0, requestsRouted: 5 },
    }),
    false,
  );
  // Flat dispatch count with work in flight: it cannot be ours.
  assert.equal(
    inferInboundWork({
      ...base,
      current: { inflight: 1, requestsRouted: 5 },
      previous: { inflight: 0, requestsRouted: 5 },
    }),
    true,
  );
  // No history yet — decline to claim anything.
  assert.equal(
    inferInboundWork({
      ...base,
      current: { inflight: 1, requestsRouted: 5 },
      previous: null,
    }),
    false,
  );
  // Nothing in flight, and a non-sharing node cannot receive inbound work.
  assert.equal(
    inferInboundWork({
      ...base,
      current: { inflight: 0, requestsRouted: 5 },
      previous: { inflight: 0, requestsRouted: 5 },
    }),
    false,
  );
  assert.equal(
    inferInboundWork({
      isSharing: false,
      current: { inflight: 1, requestsRouted: 5 },
      previous: { inflight: 0, requestsRouted: 5 },
    }),
    false,
  );
});

test("participation counts live peers, not relay notes", () => {
  const model = deriveMeshDetailModel({
    view: view({ peers: [peer(), peer({ id: "p2", label: "mac.lan" })] }),
    snapshot: snapshot(),
    usage: usage(),
    isSharing: true,
    inboundWork: false,
  });
  assert.equal(model.participationLabel, "2 peers connected");
  assert.equal(model.connected, true);
  // 115 self + 64 + 64 peers.
  assert.equal(model.capacityLabel, "243 GB");
});

test("connected but alone is distinct from not participating", () => {
  const alone = deriveMeshDetailModel({
    view: view({ peers: [] }),
    snapshot: snapshot(),
    usage: usage(),
    isSharing: true,
    inboundWork: false,
  });
  assert.equal(alone.participationLabel, "No other devices yet");
  assert.equal(alone.connected, true);

  // With no runtime, peers are unknowable and the relay view is all we have.
  const off = deriveMeshDetailModel({
    view: { connected: false, selfCapacityGb: null, peers: [] },
    snapshot: snapshot({ sharingDeviceCount: 3 }),
    usage: usage(),
    isSharing: false,
    inboundWork: false,
  });
  assert.equal(off.participationLabel, "3 sharing in this community");
  assert.equal(off.connected, false);
});

test("consuming peers add presence but never invented capacity", () => {
  const model = deriveMeshDetailModel({
    view: view({
      selfCapacityGb: null,
      peers: [peer({ capacityGb: null, state: "consuming", models: [] })],
    }),
    snapshot: snapshot(),
    usage: usage(),
    isSharing: false,
    inboundWork: false,
  });
  assert.equal(model.participationLabel, "1 peer connected");
  // Nobody reported a figure, so no GB is claimed rather than "0 GB".
  assert.equal(model.capacityLabel, null);
});

test("a null view and snapshot are renderable and claim nothing", () => {
  const model = deriveMeshDetailModel({
    view: null,
    snapshot: null,
    usage: null,
    isSharing: false,
    inboundWork: false,
  });
  assert.equal(model.capacityLabel, null);
  assert.equal(model.connected, false);
  assert.equal(model.busyNow, false);
  assert.equal(model.activityLabel, null);
  assert.equal(model.originLabel, null);
  assert.equal(model.modelCount, 0);
});

test("capacity formatting keeps small figures meaningful", () => {
  assert.equal(
    deriveMeshDetailModel({
      view: view({ selfCapacityGb: 4.62, peers: [] }),
      snapshot: snapshot(),
      usage: usage(),
      isSharing: true,
      inboundWork: false,
    }).capacityLabel,
    "4.6 GB",
  );
});

test("the card hint distinguishes giving, taking, and neither", () => {
  assert.equal(
    describeParticipationHint({
      isSharing: false,
      isConsuming: false,
      inboundWork: false,
      usage: usage(),
    }),
    "Share compute to run models",
  );
  assert.equal(
    describeParticipationHint({
      isSharing: true,
      isConsuming: false,
      inboundWork: false,
      usage: usage(),
    }),
    "You're sharing",
  );
  assert.equal(
    describeParticipationHint({
      isSharing: true,
      isConsuming: false,
      inboundWork: true,
      usage: usage({ inflight: 1 }),
    }),
    "You're sharing · serving another member",
  );
  assert.equal(
    describeParticipationHint({
      isSharing: false,
      isConsuming: true,
      inboundWork: false,
      usage: usage({ remotelyServed: 3, locallyServed: 1 }),
    }),
    "3 of 4 requests ran on shared compute",
  );
});

test("the cold popover explains the switch instead of restating a zero", () => {
  // "0 sharing in this community" is a status line with no status. Someone
  // opening this for the first time needs to know what the switch beside it
  // does, which is the one thing a zero cannot tell them.
  const model = deriveMeshDetailModel({
    view: null,
    snapshot: snapshot({
      sharingDeviceCount: 0,
      sharedCapacityGb: null,
      devices: [],
    }),
    usage: null,
    isSharing: false,
    inboundWork: false,
  });
  assert.match(model.participationLabel, /Share your spare compute/);
  assert.doesNotMatch(model.participationLabel, /^0 /);
});
