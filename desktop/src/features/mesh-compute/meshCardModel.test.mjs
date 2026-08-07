/**
 * Contract tests for the sidebar shared-compute card model.
 *
 * The card's whole job is to make "am I giving compute / taking compute /
 * neither" unmistakable, so these tests pin the distinctions that are easy to
 * regress:
 *
 *   - consuming must never render as sharing (both report state:"running")
 *   - unknown capacity must never print "0 GB"
 *   - a serve node with no advertised model is warming up, not serving
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  describeMeshCapacity,
  describeMeshHeadline,
  describeReadyModels,
  describeStartupHeadline,
  describeStartupStage,
  deriveMeshCardModel,
  formatCapacityGb,
  shortModelLabel,
} from "./meshCardModel.ts";

const OFF_TOGGLE = {
  isSharing: false,
  isConsuming: false,
  slotOccupied: false,
};
const SHARING_TOGGLE = {
  isSharing: true,
  isConsuming: false,
  slotOccupied: true,
};
const CONSUMING_TOGGLE = {
  isSharing: false,
  isConsuming: true,
  slotOccupied: true,
};

function device(overrides = {}) {
  return {
    deviceId: "endpoint-1",
    label: "Studio",
    capacityGb: 36,
    models: ["unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M"],
    state: "serving",
    isSelf: false,
    ...overrides,
  };
}

function snapshot(overrides = {}) {
  return {
    sharingDeviceCount: 1,
    sharedCapacityGb: 36,
    models: ["unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M"],
    devices: [device()],
    includesSelf: false,
    memberCount: 1,
    reason: null,
    ...overrides,
  };
}

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

function derive(overrides = {}) {
  return deriveMeshCardModel({
    snapshot: snapshot(),
    status: null,
    toggle: OFF_TOGGLE,
    pendingAction: null,
    canShare: true,
    view: null,
    usage: usage(),
    inboundWork: false,
    downloadProgress: null,
    ...overrides,
  });
}

test("capacity headline counts devices and sums reported memory", () => {
  assert.equal(
    describeMeshCapacity(
      snapshot({ sharingDeviceCount: 3, sharedCapacityGb: 42.3 }),
    ),
    "42 GB · 3 devices",
  );
  assert.equal(
    describeMeshCapacity(
      snapshot({ sharingDeviceCount: 1, sharedCapacityGb: 18 }),
    ),
    "18 GB · 1 device",
  );
});

test("unknown capacity degrades to a device count, never 0 GB", () => {
  const text = describeMeshCapacity(
    snapshot({ sharingDeviceCount: 3, sharedCapacityGb: null }),
  );
  assert.equal(text, "Mesh capacity · 3 devices");
  assert.ok(!text.includes("0 GB"), "must not claim zero capacity");
});

test("an empty mesh reads as an honest empty state", () => {
  assert.equal(
    describeMeshCapacity(
      snapshot({ sharingDeviceCount: 0, sharedCapacityGb: null }),
    ),
    "No mesh capacity yet",
  );
  // `null` is "not fetched yet" and must not be reported as an empty mesh:
  // that would pass judgement on everyone else's machines during startup.
  assert.equal(describeMeshCapacity(null), "Checking mesh capacity…");
});

test("capacity formatting keeps small figures meaningful", () => {
  assert.equal(formatCapacityGb(42.3), "42 GB");
  assert.equal(formatCapacityGb(4.62), "4.6 GB");
});

test("model labels shorten to fit a narrow sidebar", () => {
  assert.equal(
    shortModelLabel("unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M"),
    "Gemma 4 26B A4B",
  );
  assert.equal(
    shortModelLabel("unsloth/gemma-4-E4B-it-GGUF:Q4_K_M"),
    "Gemma 4 E4B",
  );
});

test("ready models collapse to a count past one", () => {
  assert.equal(describeReadyModels(snapshot()), "Gemma 4 26B A4B ready");
  assert.equal(
    describeReadyModels(snapshot({ models: ["a", "b"] })),
    "2 models ready",
  );
  assert.equal(describeReadyModels(snapshot({ models: [] })), null);
});

test("consuming never renders as sharing", () => {
  const model = derive({
    toggle: CONSUMING_TOGGLE,
    usage: usage({ remotelyServed: 4 }),
  });
  assert.equal(model.tone, "consuming");
  assert.equal(model.switchOn, false, "the Share switch must stay off");
  // The headline names the mesh; the detail line carries this machine's role,
  // and must say we are taking rather than giving.
  assert.match(model.detail, /ran on shared compute/);
  assert.ok(
    !/You're sharing/.test(model.detail),
    `consuming must not read as sharing: ${model.detail}`,
  );
  // A consuming client may be replaced by a serve runtime, so the switch stays
  // usable rather than locked.
  assert.equal(model.switchDisabled, false);
});

const SERVE_STATUS = {
  state: "running",
  mode: "serve",
  health: { status: "ok", reason: null },
  modelId: "unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M",
  modelName: null,
  apiBaseUrl: null,
  consoleUrl: null,
};

test("the headline names MeshLLM, live capacity and peer count", () => {
  assert.equal(
    describeMeshHeadline({
      view: {
        connected: true,
        selfCapacityGb: 115,
        peers: [peer({ capacityGb: 64 })],
      },
      snapshot: snapshot(),
    }),
    "MeshLLM · 179 GB, 1 peer",
  );
});

test("a consuming-only mesh shows peers without inventing capacity", () => {
  // Client-mode peers share no memory, so a mesh of consumers has no GB to
  // report — but the peers are still really there.
  assert.equal(
    describeMeshHeadline({
      view: {
        connected: true,
        selfCapacityGb: null,
        peers: [peer({ capacityGb: null, state: "consuming" })],
      },
      snapshot: snapshot(),
    }),
    "MeshLLM · 1 peer",
  );
});

test("with no runtime the headline falls back to the relay snapshot", () => {
  // Peers are unknowable without a node, so the community view is all we have.
  assert.equal(
    describeMeshHeadline({
      view: { connected: false, selfCapacityGb: null, peers: [] },
      snapshot: snapshot({ sharingDeviceCount: 3, sharedCapacityGb: 42 }),
    }),
    "42 GB · 3 devices",
  );
});

test("sharing leads with the local role and names the hosted model", () => {
  const model = derive({
    toggle: SHARING_TOGGLE,
    status: SERVE_STATUS,
    view: { connected: true, selfCapacityGb: 115, peers: [peer()] },
  });
  assert.equal(model.tone, "sharing");
  assert.equal(model.switchOn, true);
  assert.equal(model.headline, "You’re sharing compute.");
  assert.equal(model.detail, "Gemma 4 26B A4B");
  assert.equal(model.showSoloHint, false);
});

test("sharing keeps the model line stable while requests are active", () => {
  const model = derive({
    toggle: SHARING_TOGGLE,
    status: SERVE_STATUS,
    usage: usage({ inflight: 1 }),
    inboundWork: true,
  });
  assert.equal(model.detail, "Gemma 4 26B A4B");
});

test("a solo sharer is a live-view fact, not a lone relay note", () => {
  // One status note may simply be a stale one; only gossip can say "alone".
  const alone = derive({
    toggle: SHARING_TOGGLE,
    status: SERVE_STATUS,
    view: { connected: true, selfCapacityGb: 115, peers: [] },
  });
  assert.equal(alone.isSolo, true);
  const populated = derive({
    toggle: SHARING_TOGGLE,
    status: SERVE_STATUS,
    view: { connected: true, selfCapacityGb: 115, peers: [peer()] },
  });
  assert.equal(populated.isSolo, false);
});

test("a serve node with no advertised model is warming up, not serving", () => {
  const model = derive({
    toggle: SHARING_TOGGLE,
    status: {
      state: "running",
      mode: "serve",
      health: { status: "ok", reason: null },
      modelId: "m",
      modelName: null,
      apiBaseUrl: null,
      consoleUrl: null,
    },
    snapshot: snapshot({
      sharingDeviceCount: 0,
      devices: [device({ isSelf: true, state: "loading", models: [] })],
    }),
  });
  assert.equal(model.tone, "pending");
  assert.match(model.headline, /Starting to share/);
});

test("a failed runtime surfaces its reason instead of claiming to share", () => {
  const model = derive({
    toggle: SHARING_TOGGLE,
    status: {
      state: "running",
      mode: "serve",
      health: { status: "degraded", reason: "llama runtime exited" },
      modelId: "m",
      modelName: null,
      apiBaseUrl: null,
      consoleUrl: null,
    },
  });
  assert.equal(model.tone, "failed");
  assert.match(model.detail, /llama runtime exited/);
});

test("the idle invitation leads with what the community already has", () => {
  const model = derive({
    snapshot: snapshot({ sharingDeviceCount: 2, sharedCapacityGb: 54 }),
  });
  assert.equal(model.tone, "idle");
  assert.equal(model.headline, "54 GB · 2 devices");
});

test("the switch is disabled until a model can be resolved", () => {
  assert.equal(derive({ canShare: false }).switchDisabled, true);
  assert.equal(derive({ canShare: true }).switchDisabled, false);
});

test("an unknown runtime occupant is not silently replaceable", () => {
  const model = derive({
    toggle: { isSharing: false, isConsuming: false, slotOccupied: true },
  });
  assert.equal(model.switchDisabled, true);
});

function progress(overrides = {}) {
  return {
    label: "gemma",
    file: "model.gguf",
    downloadedBytes: null,
    totalBytes: null,
    status: "downloading",
    done: false,
    ...overrides,
  };
}

test("a download is named and measured, not hidden behind 'Starting…'", () => {
  // Minutes of unlabelled spinner reads as a hang; the download is the longest
  // and most opaque startup step, so it says so.
  assert.equal(
    describeStartupHeadline(
      progress({ downloadedBytes: 5e9, totalBytes: 17e9 }),
    ),
    "Downloading model · 29%",
  );
  assert.match(
    describeStartupStage(
      progress({ downloadedBytes: 5e9, totalBytes: 17e9 }),
      "fallback-model",
    ),
    /Gemma · 5 GB of 17 GB/,
  );
});

test("a download with no known total says so rather than faking 0%", () => {
  assert.equal(describeStartupHeadline(progress()), "Downloading model…");
  assert.equal(
    describeStartupHeadline(progress({ downloadedBytes: 1e9, totalBytes: 0 })),
    "Downloading model…",
  );
});

test("each startup stage is distinguishable and names the model", () => {
  assert.equal(
    describeStartupHeadline(progress({ status: "preparing" })),
    "Preparing model…",
  );
  assert.equal(describeStartupHeadline(null), "Starting to share…");
  assert.equal(
    describeStartupStage(null, "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M"),
    "Loading Gemma 4 E4B into memory.",
  );
});

test("starting shows the model and download stage on the card", () => {
  const model = derive({
    pendingAction: "start",
    downloadProgress: progress({ downloadedBytes: 2e9, totalBytes: 4e9 }),
    startingModel: "fallback-model",
  });
  assert.equal(model.tone, "pending");
  assert.equal(model.headline, "Downloading model · 50%");
  assert.match(model.detail, /Gemma · 2 GB of 4 GB/);
});

test("an empty community is a call to be first, not an error", () => {
  const model = derive({
    snapshot: snapshot({ sharingDeviceCount: 0, sharedCapacityGb: null }),
  });
  assert.equal(model.tone, "idle");
  assert.match(model.detail, /Be the first to share/);
});

test("a machine that cannot host is told it can still consume", () => {
  // Saying "turn it on" to someone who cannot is a dead end; consuming still
  // works once somebody else shares.
  const model = derive({
    snapshot: snapshot({ sharingDeviceCount: 0, sharedCapacityGb: null }),
    canShare: false,
  });
  assert.match(model.detail, /too small to share, but can use shared compute/);
  assert.equal(model.switchDisabled, true);
});

test("an unfetched snapshot is not treated as an empty community", () => {
  // null is "not checked yet" — claiming "be the first" before the relay
  // answers would be a verdict on everyone else's machines.
  const model = derive({ snapshot: null });
  assert.ok(
    !/Be the first/.test(model.detail ?? ""),
    `must not claim empty before fetching: ${model.detail}`,
  );
});
