import assert from "node:assert/strict";
import test from "node:test";

import {
  bucketHexCellCount,
  capacityGbToHexCellCount,
  deriveCommunityComputeMapModel,
} from "./communityComputeMapModel.ts";
import {
  COMMUNITY_COMPUTE_TUTORIAL_FIXTURES,
  LARGE_DENSE_COMMUNITY_COMPUTE_FIXTURE,
  createLargeDenseSnapshot,
} from "./communityComputeFixtures.ts";

function device(overrides = {}) {
  return {
    deviceId: "device-1",
    label: "Studio",
    capacityGb: 32,
    models: ["gemma"],
    state: "serving",
    isSelf: false,
    memberPubkey: "member-1",
    reportedAt: 1_786_032_000,
    modelSizeGb: 18,
    ...overrides,
  };
}

function snapshot(overrides = {}) {
  return {
    sharingDeviceCount: 1,
    contributorMemberCount: 1,
    sharedCapacityGb: 32,
    allocatedCapacityGb: 18,
    models: ["gemma"],
    devices: [device()],
    includesSelf: false,
    memberCount: 4,
    reason: null,
    ...overrides,
  };
}

test("cell buckets are exactly 1, 2, 4, and 7", () => {
  assert.deepEqual(
    [-1, 1, 1.1, 2, 2.1, 4, 4.1, 999, Number.NaN].map(bucketHexCellCount),
    [1, 1, 2, 2, 4, 4, 7, 7, 1],
  );
  assert.deepEqual(
    [null, 128, 128.1, 256, 256.1, 512, 512.1, 1000].map(
      capacityGbToHexCellCount,
    ),
    [1, 1, 2, 2, 4, 4, 7, 7],
  );
});

test("only ready serving models become deployments", () => {
  const model = deriveCommunityComputeMapModel(
    snapshot({
      sharingDeviceCount: 1,
      devices: [
        device({ deviceId: "a", models: ["gemma", "qwen"] }),
        device({ deviceId: "b", state: "loading", models: [] }),
        device({ deviceId: "c", state: "standby", models: [] }),
        device({ deviceId: "d", state: "consuming", models: [] }),
      ],
    }),
  );

  assert.equal(model.status, "ready");
  assert.deepEqual(
    model.deployments.map((deployment) => deployment.modelId),
    ["gemma", "qwen"],
  );
  assert.equal(model.kpis.deploymentCount, 2);
  assert.equal(model.kpis.contributorMemberCount, 1);
  assert.equal(model.health.counts.healthy, 1);
  assert.equal(model.health.counts.warming, 1);
  assert.equal(model.health.counts.idle, 1);
});

test("one territory is produced per model and per device replica", () => {
  const model = deriveCommunityComputeMapModel(
    snapshot({
      devices: [
        device({ deviceId: "a", models: ["qwen", "gemma"] }),
        device({ deviceId: "b", memberPubkey: "member-2", models: ["gemma"] }),
      ],
    }),
  );
  assert.equal(model.deployments.length, 3);
  assert.equal(
    model.deployments.filter((deployment) => deployment.modelId === "gemma")
      .length,
    2,
  );
  assert.equal(new Set(model.deployments.map(({ id }) => id)).size, 3);
});

test("reported model footprint controls area without fixture overrides", () => {
  const model = deriveCommunityComputeMapModel(
    snapshot({ devices: [device({ capacityGb: 768, modelSizeGb: 96 })] }),
  );
  assert.equal(model.deployments[0].cellCount, 1);

  const large = deriveCommunityComputeMapModel(
    snapshot({ devices: [device({ capacityGb: 768, modelSizeGb: 646 })] }),
  );
  assert.equal(large.deployments[0].cellCount, 7);
});

test("loading, consumer-only, and empty snapshots render no territories", () => {
  for (const state of ["loading", "standby", "consuming"]) {
    const model = deriveCommunityComputeMapModel(
      snapshot({ devices: [device({ state, models: [] })] }),
    );
    assert.equal(model.status, "empty");
    assert.equal(model.deployments.length, 0);
  }
  assert.equal(deriveCommunityComputeMapModel(null).status, "loading");
  assert.equal(
    deriveCommunityComputeMapModel(
      COMMUNITY_COMPUTE_TUTORIAL_FIXTURES.empty.snapshot,
    ).status,
    "empty",
  );
});

test("projection is deterministic and independent of device/model order", () => {
  const devices = [
    device({ deviceId: "z", label: "Zulu", models: ["qwen", "gemma"] }),
    device({ deviceId: "a", label: "Alpha", models: ["mistral"] }),
  ];
  const forward = deriveCommunityComputeMapModel(snapshot({ devices }));
  const reverse = deriveCommunityComputeMapModel(
    snapshot({
      devices: [...devices]
        .reverse()
        .map((item) => ({ ...item, models: [...item.models].reverse() })),
    }),
  );
  assert.deepEqual(forward, reverse);
});

test("tutorial fixtures use only production fields and production sizing", () => {
  assert.deepEqual(Object.keys(COMMUNITY_COMPUTE_TUTORIAL_FIXTURES).sort(), [
    "empty",
    "large-dense-500",
    "single-device",
    "small-company",
  ]);
  const allowed = new Set([
    "deviceId",
    "label",
    "capacityGb",
    "models",
    "state",
    "isSelf",
    "memberPubkey",
    "reportedAt",
    "modelSizeGb",
  ]);
  for (const fixture of Object.values(COMMUNITY_COMPUTE_TUTORIAL_FIXTURES)) {
    for (const item of fixture.snapshot.devices) {
      assert.deepEqual(
        Object.keys(item).filter((key) => !allowed.has(key)),
        [],
      );
    }
  }
  const small = deriveCommunityComputeMapModel(
    COMMUNITY_COMPUTE_TUTORIAL_FIXTURES["small-company"].snapshot,
  );
  assert.equal(small.deployments.length, 12);
  assert.equal(
    small.deployments.filter(({ deviceLabel }) =>
      deviceLabel.startsWith("Team MacBook"),
    ).length,
    10,
  );
  assert.equal(
    small.deployments.filter(({ cellCount }) => cellCount === 1).length,
    10,
  );
  assert.equal(
    small.deployments.filter(({ cellCount }) => cellCount === 7).length,
    2,
  );
});

test("large dense fixture is deterministic and excludes non-serving rows", () => {
  const fixture = LARGE_DENSE_COMMUNITY_COMPUTE_FIXTURE.snapshot;
  assert.deepEqual(fixture, createLargeDenseSnapshot(500));
  assert.equal(fixture.devices.length, 500);
  const servingCount = fixture.devices.filter(
    (item) => item.state === "serving" && item.models.length > 0,
  ).length;
  const model = deriveCommunityComputeMapModel(fixture);
  assert.equal(model.deployments.length, servingCount);
  assert.equal(model.kpis.deploymentCount, servingCount);
  assert.equal(
    new Set(model.deployments.map(({ id }) => id)).size,
    servingCount,
  );
});
