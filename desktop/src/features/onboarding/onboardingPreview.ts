const ONBOARDING_PREVIEW_PARAM = "onboardingPreview";
const ONBOARDING_PREVIEW_VALUE = "1";

type OnboardingPreviewEnvironment = {
  dev: boolean;
  mode: string;
  search: string;
};

export type OnboardingPreviewPage =
  | "landing"
  | "email"
  | "identity-key"
  | "sign-in"
  | "sign-in-key"
  | "forgot-password"
  | "backup-options"
  | "identity-key-help"
  | "backup-password"
  | "setup"
  | "harness-connection-method"
  | "harness-connection"
  | "harness-connection-help"
  | "harness-connection-detail"
  | "config"
  | "community-choice"
  | "community-entry"
  | "community-connecting"
  | "community-profile"
  | "starter-team"
  | "welcome-channel"
  | "community-home";

export type OnboardingPreviewVariant = "today" | "v3";

export const ONBOARDING_PREVIEW_LANDING_ACTIONS = {
  today: {
    primary: {
      label: "Create a new identity key",
      page: "identity-key",
    },
    secondary: {
      label: "Use an existing key",
      page: "sign-in-key",
    },
  },
  v3: {
    primary: {
      label: "Create an account",
      page: "email",
    },
    secondary: {
      label: "Sign in",
      page: "sign-in",
    },
  },
} as const satisfies Record<
  OnboardingPreviewVariant,
  Record<
    "primary" | "secondary",
    { label: string; page: OnboardingPreviewPage }
  >
>;

type OnboardingPreviewJourney = {
  afterAccount: OnboardingPreviewPage;
  afterCommunityEntry: OnboardingPreviewPage;
  afterProfile: OnboardingPreviewPage;
  communityChoiceBack: OnboardingPreviewPage | null;
  communityStep: number;
  finalStep: number;
  includeExistingCommunity: boolean;
  profileStep: number;
  totalSteps: number;
};

/** Workshop-only route maps that preserve today's full flow beside V3. */
export const ONBOARDING_PREVIEW_JOURNEYS: Record<
  OnboardingPreviewVariant,
  OnboardingPreviewJourney
> = {
  today: {
    afterAccount: "setup",
    afterCommunityEntry: "community-connecting",
    afterProfile: "starter-team",
    communityChoiceBack: "config",
    communityStep: 5,
    finalStep: 7,
    includeExistingCommunity: true,
    profileStep: 6,
    totalSteps: 7,
  },
  v3: {
    afterAccount: "harness-connection",
    afterCommunityEntry: "community-profile",
    afterProfile: "community-home",
    communityChoiceBack: "harness-connection",
    communityStep: 4,
    finalStep: 5,
    includeExistingCommunity: false,
    profileStep: 5,
    totalSteps: 5,
  },
};

/** Resolve the optional V3 harness experiment without changing Today's flow. */
export function resolveOnboardingPreviewJourney(
  variant: OnboardingPreviewVariant,
  harnessConnectionInOnboarding: boolean,
): OnboardingPreviewJourney {
  const journey = ONBOARDING_PREVIEW_JOURNEYS[variant];
  if (variant !== "v3" || harnessConnectionInOnboarding) return journey;

  return {
    ...journey,
    afterAccount: "community-choice",
    communityChoiceBack: null,
    communityStep: 3,
    finalStep: 4,
    profileStep: 4,
    totalSteps: 4,
  };
}

/** Resolve the explicit, non-production onboarding workshop route. */
export function resolveOnboardingPreviewMode({
  dev,
  mode,
  search,
}: OnboardingPreviewEnvironment) {
  if (!dev && mode !== "e2e") return false;
  return (
    new URLSearchParams(search).get(ONBOARDING_PREVIEW_PARAM) ===
    ONBOARDING_PREVIEW_VALUE
  );
}

/** True only for a development or explicit E2E preview boot. */
export function onboardingPreviewRequested() {
  if (typeof window === "undefined") return false;
  return resolveOnboardingPreviewMode({
    dev: import.meta.env.DEV,
    mode: import.meta.env.MODE,
    search: window.location.search,
  });
}
