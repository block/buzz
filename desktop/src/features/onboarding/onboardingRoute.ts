export const ONBOARDING_ROUTE_STEPS = [
  "identity",
  "key-import",
  "backup",
  "backup-options",
  "backup-password",
  "agents",
  "defaults",
  "community",
  "community-existing",
  "community-join",
  "community-member",
  "community-hosted",
  "community-owned",
  "profile",
  "team",
] as const;

export type OnboardingRouteStep = (typeof ONBOARDING_ROUTE_STEPS)[number];

export type MachineOnboardingRouteStep = Extract<
  OnboardingRouteStep,
  | "identity"
  | "key-import"
  | "backup"
  | "backup-options"
  | "backup-password"
  | "agents"
  | "defaults"
>;

export type CommunitySetupRouteStep = Extract<
  OnboardingRouteStep,
  | "community"
  | "community-existing"
  | "community-join"
  | "community-member"
  | "community-hosted"
  | "community-owned"
>;

const ONBOARDING_PATH_PREFIX = "/onboarding/";
const ONBOARDING_ROUTE_STEP_SET = new Set<string>(ONBOARDING_ROUTE_STEPS);
const ONBOARDING_HISTORY_STATE_KEY = "buzzOnboarding";

export type OnboardingHistoryMarker = {
  depth: number;
  sessionId: string;
};

export type OnboardingHistoryEntry = {
  depth: number;
  step: OnboardingRouteStep;
};

export function onboardingRoutePath(step: OnboardingRouteStep) {
  return `${ONBOARDING_PATH_PREFIX}${step}` as const;
}

export function parseOnboardingRouteStep(
  pathname: string,
): OnboardingRouteStep | null {
  if (!pathname.startsWith(ONBOARDING_PATH_PREFIX)) return null;
  const step = pathname.slice(ONBOARDING_PATH_PREFIX.length);
  return ONBOARDING_ROUTE_STEP_SET.has(step)
    ? (step as OnboardingRouteStep)
    : null;
}

export function createOnboardingHistoryState(
  sessionId: string,
  depth: number,
): Record<string, OnboardingHistoryMarker> {
  return {
    [ONBOARDING_HISTORY_STATE_KEY]: {
      depth,
      sessionId,
    },
  };
}

export function readOnboardingHistoryEntry(
  pathname: string,
  state: unknown,
  sessionId: string,
): OnboardingHistoryEntry | null {
  if (!state || typeof state !== "object") return null;
  const marker = (state as Record<string, unknown>)[
    ONBOARDING_HISTORY_STATE_KEY
  ];
  if (!marker || typeof marker !== "object") return null;
  const { depth, sessionId: markerSessionId } =
    marker as Partial<OnboardingHistoryMarker>;
  if (
    markerSessionId !== sessionId ||
    typeof depth !== "number" ||
    !Number.isInteger(depth) ||
    depth < 0
  ) {
    return null;
  }
  const step = parseOnboardingRouteStep(pathname);
  return step ? { depth, step } : null;
}

export function isMachineOnboardingRouteStep(
  step: OnboardingRouteStep | null,
): step is MachineOnboardingRouteStep {
  return (
    step === "identity" ||
    step === "key-import" ||
    step === "backup" ||
    step === "backup-options" ||
    step === "backup-password" ||
    step === "agents" ||
    step === "defaults"
  );
}

export function isCommunitySetupRouteStep(
  step: OnboardingRouteStep | null,
): step is CommunitySetupRouteStep {
  return (
    step === "community" ||
    step === "community-existing" ||
    step === "community-join" ||
    step === "community-member" ||
    step === "community-hosted" ||
    step === "community-owned"
  );
}
