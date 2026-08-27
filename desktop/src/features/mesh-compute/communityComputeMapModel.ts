import type {
  MeshDeviceState,
  MeshSnapshot,
  MeshSnapshotDevice,
} from "@/shared/api/tauriMesh";

/** The only territory sizes rendered by the community compute map. */
export const HEX_CELL_BUCKETS = [1, 2, 4, 7] as const;

export type HexCellCount = (typeof HEX_CELL_BUCKETS)[number];

/**
 * The Community view intentionally consumes only fields in the production
 * snapshot contract. Tutorial fixtures use this same shape so examples cannot
 * quietly grow capabilities that live Compute does not have.
 */
export type CommunityComputeDeviceInput = MeshSnapshotDevice;

/** A readonly, structurally compatible form of the current mesh snapshot. */
export type CommunityComputeSnapshotInput<
  TDevice extends MeshSnapshotDevice = CommunityComputeDeviceInput,
> = Omit<MeshSnapshot, "devices"> & {
  readonly devices: readonly TDevice[];
};

export type CommunityComputeDeploymentHealth =
  | "healthy"
  | "warming"
  | "idle"
  | "degraded"
  | "offline"
  | "unknown";

export type CommunityComputeMapHealth =
  | "healthy"
  | "attention"
  | "critical"
  | "empty"
  | "unknown";

export type CommunityComputeDeployment<
  TDevice extends MeshSnapshotDevice = CommunityComputeDeviceInput,
> = {
  /** Stable map identity. Never derived from the device's array position. */
  id: string;
  /** Model shown as the territory's primary identity. */
  label: string;
  modelId: string;
  deviceLabel: string;
  capacityGb: number | null;
  models: readonly string[];
  state: MeshDeviceState;
  isSelf: boolean;
  health: CommunityComputeDeploymentHealth;
  healthReason: string | null;
  cellCount: HexCellCount;
  /** Preserves additive backend data for a future detail projection. */
  source: Readonly<TDevice>;
};

export type CommunityComputeKpis = {
  memberCount: number;
  contributorMemberCount: number;
  deploymentCount: number;
  activeDeploymentCount: number;
  sharedCapacityGb: number | null;
  allocatedCapacityGb: number | null;
  allocatedPercent: number | null;
  modelCount: number;
};

export type CommunityComputeHealthSummary = {
  status: CommunityComputeMapHealth;
  label: string;
  reason: string | null;
  counts: Record<CommunityComputeDeploymentHealth, number>;
};

export type CommunityComputeMapModel<
  TDevice extends MeshSnapshotDevice = CommunityComputeDeviceInput,
> = {
  status: "loading" | "ready" | "empty";
  kpis: CommunityComputeKpis;
  health: CommunityComputeHealthSummary;
  deployments: readonly CommunityComputeDeployment<TDevice>[];
};

/**
 * Round a desired territory footprint up to one of the four supported shapes.
 * Invalid and non-positive weights still get one cell so a deployment cannot
 * disappear from the map.
 */
export function bucketHexCellCount(
  weight: number | null | undefined,
): HexCellCount {
  if (weight === null || weight === undefined || !Number.isFinite(weight)) {
    return 1;
  }
  const normalized = Math.max(1, Math.ceil(weight));
  return HEX_CELL_BUCKETS.find((bucket) => normalized <= bucket) ?? 7;
}

/**
 * Turn model memory into a deliberately coarse footprint.
 *
 * A 128 GB Mac is the common mesh workhorse, so every model that fits on one is
 * one cell. Multi-cell territories are reserved for genuinely multi-GPU scale
 * deployments; this avoids making ordinary local models look fragmented.
 */
export function capacityGbToHexCellCount(
  capacityGb: number | null | undefined,
): HexCellCount {
  if (capacityGb === null || capacityGb === undefined || capacityGb <= 128) {
    return 1;
  }
  if (capacityGb <= 256) {
    return 2;
  }
  if (capacityGb <= 512) {
    return 4;
  }
  return 7;
}

const EMPTY_HEALTH_COUNTS: Record<CommunityComputeDeploymentHealth, number> = {
  healthy: 0,
  warming: 0,
  idle: 0,
  degraded: 0,
  offline: 0,
  unknown: 0,
};

/** Project a backend snapshot into the complete, render-agnostic map model. */
export function deriveCommunityComputeMapModel<
  TDevice extends MeshSnapshotDevice = CommunityComputeDeviceInput,
>(
  snapshot: CommunityComputeSnapshotInput<TDevice> | null,
): CommunityComputeMapModel<TDevice> {
  if (snapshot === null) {
    return {
      status: "loading",
      kpis: {
        memberCount: 0,
        contributorMemberCount: 0,
        deploymentCount: 0,
        activeDeploymentCount: 0,
        sharedCapacityGb: null,
        allocatedCapacityGb: null,
        allocatedPercent: null,
        modelCount: 0,
      },
      health: {
        status: "unknown",
        label: "Checking community compute",
        reason: null,
        counts: { ...EMPTY_HEALTH_COUNTS },
      },
      deployments: [],
    };
  }

  const deployments = projectDeployments(snapshot.devices);
  const healthStates = snapshot.devices
    .filter((device) => device.state !== "consuming")
    .map((device) => healthFromState(device.state));
  const counts = countHealth(healthStates);
  const models = new Set(snapshot.models);
  for (const deployment of deployments) {
    for (const model of deployment.models) {
      models.add(model);
    }
  }

  const contributorMemberCount =
    snapshot.contributorMemberCount ??
    new Set(
      deployments
        .filter((deployment) => deployment.state === "serving")
        .map(
          (deployment) =>
            deployment.source.memberPubkey?.trim() || deployment.id,
        ),
    ).size;
  const sharedCapacityGb = finiteNonNegative(snapshot.sharedCapacityGb);
  const reportedAllocated = finiteNonNegative(snapshot.allocatedCapacityGb);
  const servingDeployments = deployments.filter(
    (deployment) => deployment.state === "serving",
  );
  const inferredAllocations = servingDeployments.map((deployment) =>
    finiteNonNegative(deployment.source.modelSizeGb),
  );
  const allocatedCapacityGb =
    reportedAllocated ??
    (inferredAllocations.length > 0 &&
    inferredAllocations.every((value) => value !== null)
      ? inferredAllocations.reduce((total, value) => total + (value ?? 0), 0)
      : null);

  return {
    status: deployments.length === 0 ? "empty" : "ready",
    kpis: {
      memberCount: nonNegativeInteger(snapshot.memberCount),
      contributorMemberCount,
      deploymentCount: deployments.length,
      activeDeploymentCount: counts.healthy + counts.warming,
      sharedCapacityGb,
      allocatedCapacityGb,
      allocatedPercent:
        allocatedCapacityGb !== null &&
        sharedCapacityGb !== null &&
        sharedCapacityGb > 0
          ? Math.min(100, (allocatedCapacityGb / sharedCapacityGb) * 100)
          : null,
      modelCount: models.size,
    },
    health: summarizeHealth(counts, healthStates, snapshot.reason),
    deployments,
  };
}

function projectDeployments<TDevice extends MeshSnapshotDevice>(
  devices: readonly TDevice[],
): CommunityComputeDeployment<TDevice>[] {
  const canonical = devices
    .filter((device) => device.state === "serving" && device.models.length > 0)
    .flatMap((device) =>
      [...new Set(device.models)].map((model) => ({
        device: { ...device, models: [model] } as TDevice,
        identity: deploymentIdentity(device, model),
      })),
    )
    .sort(
      (left, right) =>
        left.identity.localeCompare(right.identity) ||
        deviceFingerprint(left.device).localeCompare(
          deviceFingerprint(right.device),
        ),
    );
  const occurrences = new Map<string, number>();

  return canonical.map(({ device, identity }) => {
    const occurrence = occurrences.get(identity) ?? 0;
    occurrences.set(identity, occurrence + 1);
    const models = [...new Set(device.models)].sort();
    return {
      id: occurrence === 0 ? identity : `${identity}#${occurrence + 1}`,
      label: models[0] ?? device.label,
      modelId: models[0] ?? "Model not reported",
      deviceLabel: device.label,
      capacityGb: finiteNonNegative(device.capacityGb),
      models,
      state: device.state,
      isSelf: device.isSelf,
      health: healthFromState(device.state),
      healthReason: null,
      cellCount: capacityGbToHexCellCount(
        device.modelSizeGb ?? device.capacityGb,
      ),
      source: device,
    };
  });
}

function deploymentIdentity(
  device: MeshSnapshotDevice,
  modelId: string,
): string {
  const deviceId = cleanOptionalText(device.deviceId);
  const model = encodeURIComponent(modelId);
  return deviceId === null
    ? `derived:${encodeURIComponent(deviceFingerprint(device))}:${model}`
    : `deployment:${encodeURIComponent(deviceId)}:${model}`;
}

function deviceFingerprint(device: MeshSnapshotDevice): string {
  return JSON.stringify([
    device.label,
    device.capacityGb,
    [...device.models].sort(),
    device.state,
    device.isSelf,
  ]);
}

function healthFromState(
  state: MeshDeviceState,
): CommunityComputeDeploymentHealth {
  switch (state) {
    case "serving":
    case "consuming":
      return "healthy";
    case "loading":
      return "warming";
    case "standby":
      return "idle";
  }
}

function countHealth(
  states: readonly CommunityComputeDeploymentHealth[],
): Record<CommunityComputeDeploymentHealth, number> {
  const counts = { ...EMPTY_HEALTH_COUNTS };
  for (const state of states) {
    counts[state] += 1;
  }
  return counts;
}

function summarizeHealth(
  counts: Record<CommunityComputeDeploymentHealth, number>,
  states: readonly CommunityComputeDeploymentHealth[],
  snapshotReason: string | null,
): CommunityComputeHealthSummary {
  if (states.length === 0) {
    return {
      status: "empty",
      label: "No deployments",
      reason: cleanOptionalText(snapshotReason),
      counts,
    };
  }

  const unavailable = counts.offline + counts.degraded;
  const reason = null;

  if (counts.offline === states.length) {
    return { status: "critical", label: "Compute unavailable", reason, counts };
  }
  if (unavailable > 0) {
    return {
      status: "attention",
      label: `${unavailable} deployment${unavailable === 1 ? "" : "s"} need attention`,
      reason,
      counts,
    };
  }
  if (counts.unknown === states.length) {
    return { status: "unknown", label: "Health unknown", reason: null, counts };
  }
  return {
    status: "healthy",
    label: "No reported issues",
    reason: null,
    counts,
  };
}

function finiteNonNegative(value: number | null | undefined): number | null {
  return value !== null &&
    value !== undefined &&
    Number.isFinite(value) &&
    value >= 0
    ? value
    : null;
}

function nonNegativeInteger(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0;
}

function cleanOptionalText(value: string | null | undefined): string | null {
  const cleaned = value?.trim();
  return cleaned ? cleaned : null;
}
