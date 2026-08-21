const FORCE_FRESH_VALUE = "true";

/**
 * Development-only switch for replaying first-run onboarding with an existing
 * identity. Vite removes the DEV branch from production builds.
 */
export const forceFreshOnboarding =
  import.meta.env?.DEV === true &&
  import.meta.env.VITE_BUZZ_FORCE_FRESH_ONBOARDING === FORCE_FRESH_VALUE;

const devBootId = forceFreshOnboarding ? crypto.randomUUID() : null;

/** Give each forced-fresh webview boot its own relay marker namespace. */
export function welcomeKickoffMarker(baseMarker: string) {
  return devBootId ? `${baseMarker}.dev-${devBootId}` : baseMarker;
}
