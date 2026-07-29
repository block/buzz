/**
 * Pure page-graph data for the machine onboarding flow (onboarding v2).
 *
 * State names are a cross-PR contract — the advanced (no-email) entry PR
 * builds against `advanced-identity`, and the signup/login PR fills `signup`
 * and `login`. Do not rename without coordinating in #buzz-onboarding-v2.
 */

export type MachineOnboardingPage =
  /** Landing screen: wordmark + tagline + entry CTAs. Renders no chrome. */
  | "identity"
  /** Standard flow: email + password signup (placeholder until PR 3). */
  | "signup"
  /** Returning users: email + password login (placeholder until PR 3). */
  | "login"
  /**
   * Advanced (no-email) flow behind "Sign up without email": create/import
   * key choice. Placeholder until the advanced-entry PR fills it.
   */
  | "advanced-identity"
  | "key-import"
  | "backup"
  | "setup"
  | "config";

/**
 * Position of a machine-flow page on the shared onboarding pagination track
 * (`OnboardingChrome`), or `null` for the landing screen, which renders no
 * chrome. The account-entry screens (signup / login / advanced-identity)
 * share position 1; key screens share position 2 — the track counts flow
 * positions, not branches.
 */
export function machineOnboardingChromePosition(
  page: MachineOnboardingPage,
): number | null {
  switch (page) {
    case "identity":
      return null;
    case "signup":
    case "login":
    case "advanced-identity":
      return 1;
    case "key-import":
    case "backup":
      return 2;
    case "setup":
      return 3;
    case "config":
      return 4;
  }
}

/**
 * Back target from the setup (providers) step. Imported identities skip the
 * backup step on the way in (the user already has their key saved), so Back
 * must return to key-import — never surface backup for an imported key.
 */
export function machineSetupBackTarget(
  identityWasImported: boolean,
): MachineOnboardingPage {
  return identityWasImported ? "key-import" : "backup";
}
