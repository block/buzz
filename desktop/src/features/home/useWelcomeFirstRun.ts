import * as React from "react";

/**
 * First-run welcome visibility (onboarding step 7a).
 *
 * The welcome empty state shows the first time a freshly-onboarded user lands
 * on Home and stays until they dismiss it (per product decision: it persists
 * rather than being a one-shot flash). Dismissal is keyed per-pubkey so each
 * identity sees it once.
 */
const WELCOME_DISMISSED_STORAGE_KEY = "buzz-home-welcome-dismissed.v1";

function dismissedStorageKey(pubkey: string) {
  return `${WELCOME_DISMISSED_STORAGE_KEY}:${pubkey}`;
}

function readDismissed(pubkey: string | null | undefined) {
  if (typeof window === "undefined" || !pubkey) {
    return true; // no pubkey → nothing to welcome
  }
  return window.localStorage.getItem(dismissedStorageKey(pubkey)) === "true";
}

export function useWelcomeFirstRun(pubkey: string | null | undefined) {
  const [dismissed, setDismissed] = React.useState(() => readDismissed(pubkey));

  // Re-read when the active identity changes (workspace switch / re-login).
  React.useEffect(() => {
    setDismissed(readDismissed(pubkey));
  }, [pubkey]);

  const dismiss = React.useCallback(() => {
    if (typeof window !== "undefined" && pubkey) {
      window.localStorage.setItem(dismissedStorageKey(pubkey), "true");
    }
    setDismissed(true);
  }, [pubkey]);

  return { showWelcome: !dismissed, dismiss };
}

/** Reset for workspace-switch teardown (see resetWorkspaceState). */
export function clearWelcomeFirstRunState() {
  if (typeof window === "undefined") {
    return;
  }
  for (const key of Object.keys(window.localStorage)) {
    if (key.startsWith(`${WELCOME_DISMISSED_STORAGE_KEY}:`)) {
      window.localStorage.removeItem(key);
    }
  }
}
