import type {
  CommunityComputeDeviceInput,
  CommunityComputeSnapshotInput,
} from "./communityComputeMapModel";

export type CommunityComputeTutorialFixtureId =
  | "empty"
  | "single-device"
  | "small-company"
  | "large-dense-500";

export type CommunityComputeTutorialFixture = {
  id: CommunityComputeTutorialFixtureId;
  title: string;
  description: string;
  snapshot: CommunityComputeSnapshotInput;
};

const MODEL_REFS = [
  "unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M",
  "Qwen/Qwen3.5-35B-A3B-GGUF:Q4_K_M",
  "mistralai/Ministral-3-14B-Instruct-GGUF:Q4_K_M",
  "bartowski/Llama-3.3-70B-Instruct-GGUF:Q4_K_M",
] as const;

export const EMPTY_COMMUNITY_COMPUTE_FIXTURE: CommunityComputeTutorialFixture =
  {
    id: "empty",
    title: "New community",
    description: "No member has shared compute yet.",
    snapshot: {
      sharingDeviceCount: 0,
      contributorMemberCount: 0,
      sharedCapacityGb: null,
      allocatedCapacityGb: null,
      models: [],
      devices: [],
      includesSelf: false,
      memberCount: 8,
      reason: "No sharing devices found",
    },
  };

export const SINGLE_DEVICE_COMMUNITY_COMPUTE_FIXTURE: CommunityComputeTutorialFixture =
  {
    id: "single-device",
    title: "First deployment",
    description: "One member is sharing a single workstation.",
    snapshot: snapshotFromDevices(
      [
        device({
          index: 0,
          id: "studio-workstation",
          label: "Morgan’s Studio",
          capacityGb: 24,
          modelSizeGb: 18,
          isSelf: true,
        }),
      ],
      1,
    ),
  };

export const SMALL_COMPANY_COMMUNITY_COMPUTE_FIXTURE: CommunityComputeTutorialFixture =
  {
    id: "small-company",
    title: "Small company",
    description: "A mixed fleet across a small distributed team.",
    snapshot: snapshotFromDevices(
      [
        ...[
          { capacityGb: 24, modelSizeGb: 18 },
          { capacityGb: 24, modelSizeGb: 18 },
          { capacityGb: 36, modelSizeGb: 28 },
          { capacityGb: 36, modelSizeGb: 28 },
          { capacityGb: 48, modelSizeGb: 36 },
          { capacityGb: 48, modelSizeGb: 36 },
          { capacityGb: 64, modelSizeGb: 48 },
          { capacityGb: 64, modelSizeGb: 48 },
          { capacityGb: 96, modelSizeGb: 72 },
          { capacityGb: 96, modelSizeGb: 72 },
        ].map(({ capacityGb, modelSizeGb }, index) =>
          device({
            index,
            id: `team-macbook-${String(index + 1).padStart(2, "0")}`,
            label: `Team MacBook ${String(index + 1).padStart(2, "0")}`,
            capacityGb,
            modelSizeGb,
            isSelf: index === 0,
          }),
        ),
        device({
          index: 10,
          id: "intensive-model-host-a",
          label: "Intensive model host A",
          capacityGb: 768,
          modelRef: "unsloth/Kimi-K2-Thinking-GGUF:UD-Q4_K_XL",
          modelSizeGb: 646,
        }),
        device({
          index: 11,
          id: "intensive-model-host-b",
          label: "Intensive model host B",
          capacityGb: 1_440,
          modelRef: "unsloth/DeepSeek-V3.2-GGUF:UD-Q4_K_XL",
          modelSizeGb: 1_000,
        }),
      ],
      24,
    ),
  };

/** Deterministic stress fixture using only the production snapshot contract. */
export const LARGE_DENSE_COMMUNITY_COMPUTE_FIXTURE: CommunityComputeTutorialFixture =
  {
    id: "large-dense-500",
    title: "Large company",
    description:
      "Five hundred deterministic reporting devices for density testing.",
    snapshot: createLargeDenseSnapshot(500),
  };

export const COMMUNITY_COMPUTE_TUTORIAL_FIXTURES: Readonly<
  Record<CommunityComputeTutorialFixtureId, CommunityComputeTutorialFixture>
> = {
  empty: EMPTY_COMMUNITY_COMPUTE_FIXTURE,
  "single-device": SINGLE_DEVICE_COMMUNITY_COMPUTE_FIXTURE,
  "small-company": SMALL_COMPANY_COMMUNITY_COMPUTE_FIXTURE,
  "large-dense-500": LARGE_DENSE_COMMUNITY_COMPUTE_FIXTURE,
};

export function createLargeDenseSnapshot(
  deviceCount = 500,
): CommunityComputeSnapshotInput {
  const count = Math.max(0, Math.floor(deviceCount));
  const capacities = [8, 16, 24, 32, 48, 64, 96, 128] as const;
  const devices = Array.from({ length: count }, (_, index) => {
    const capacityGb = capacities[index % capacities.length];
    const state =
      index % 41 === 0 ? "loading" : index % 17 === 0 ? "standby" : "serving";
    const largeIndex = [49, 149, 249, 349, 449, 499].indexOf(index);
    const largeSizes = [170, 230, 350, 480, 700, 1_000] as const;
    const largeCapacities = [192, 256, 384, 512, 768, 1_440] as const;
    const modelSizeGb =
      largeIndex >= 0
        ? largeSizes[largeIndex]
        : roundCapacity(capacityGb * 0.72);
    return device({
      index,
      id: `dense-device-${String(index + 1).padStart(4, "0")}`,
      label:
        largeIndex >= 0
          ? `${largeCapacities[largeIndex]} GB model host`
          : `Compute ${String(index + 1).padStart(4, "0")}`,
      capacityGb: largeIndex >= 0 ? largeCapacities[largeIndex] : capacityGb,
      modelRef:
        largeIndex >= 0
          ? `community/Large-${modelSizeGb}GB-Instruct:FP8`
          : undefined,
      modelSizeGb,
      state,
      models: state === "serving" ? undefined : [],
      isSelf: index === 0,
    });
  });
  return snapshotFromDevices(devices, Math.ceil(count * 1.37));
}

type DeviceOptions = {
  index: number;
  id: string;
  label: string;
  capacityGb: number;
  modelRef?: string;
  modelSizeGb?: number;
  models?: string[];
  state?: CommunityComputeDeviceInput["state"];
  isSelf?: boolean;
};

function device(options: DeviceOptions): CommunityComputeDeviceInput {
  const model =
    options.modelRef ?? MODEL_REFS[options.index % MODEL_REFS.length];
  return {
    deviceId: options.id,
    label: options.label,
    capacityGb: options.capacityGb,
    models: options.models ?? [model],
    state: options.state ?? "serving",
    isSelf: options.isSelf ?? false,
    memberPubkey: `tutorial-member-${String(options.index + 1).padStart(4, "0")}`,
    reportedAt: 1_786_032_000,
    modelSizeGb: options.modelSizeGb ?? null,
  };
}

function roundCapacity(value: number): number {
  return Math.round(value * 10) / 10;
}

function snapshotFromDevices(
  devices: CommunityComputeDeviceInput[],
  memberCount: number,
): CommunityComputeSnapshotInput {
  const serving = devices.filter(
    (item) => item.state === "serving" && item.models.length > 0,
  );
  const models = [...new Set(serving.flatMap((item) => item.models))].sort();
  const sharedCapacityGb = completeSum(serving.map((item) => item.capacityGb));
  const allocatedCapacityGb = completeSum(
    serving.map((item) => item.modelSizeGb ?? null),
  );
  return {
    sharingDeviceCount: serving.length,
    contributorMemberCount: new Set(
      serving.map((item) => item.memberPubkey ?? item.deviceId),
    ).size,
    sharedCapacityGb,
    allocatedCapacityGb,
    models,
    devices,
    includesSelf: serving.some((item) => item.isSelf),
    memberCount,
    observedAt: 1_786_032_000,
    freshnessSeconds: 120,
    reason: serving.length === 0 ? "No sharing devices found" : null,
  };
}

function completeSum(values: readonly (number | null)[]): number | null {
  return values.length > 0 && values.every((value) => value !== null)
    ? values.reduce<number>((total, value) => total + (value ?? 0), 0)
    : null;
}
