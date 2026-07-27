import { isTauri } from "@tauri-apps/api/core";

import { invokeTauri } from "@/shared/api/tauri";
import type { Identity } from "@/shared/api/types";

type RawIdentity = {
  pubkey: string;
  display_name: string;
  lost?: boolean;
  locked?: boolean;
  reset_failed?: boolean;
};

function fromRawIdentity(raw: RawIdentity): Identity {
  return {
    pubkey: raw.pubkey,
    displayName: raw.display_name,
    lost: raw.lost === true,
    locked: raw.locked === true,
    resetFailed: raw.reset_failed === true,
  };
}

// Web deployments serve this same bundle outside the Tauri shell, where
// there is no native keychain-backed identity at all. Without this guard,
// get_identity fell through to invokeTauri, which throws deep inside
// @tauri-apps/api/core's invoke() — the identity query never settled to a
// clean "error" (it hung on "pending"/"paused" indefinitely), which kept
// useMachineOnboardingState() stuck on the "blocking" stage forever. Failing
// fast here lets identityQuery.status reach "error" immediately, which
// machineOnboarding.ts already treats as a signal to proceed to "ready".
function requireTauri(command: string): void {
  if (!isTauri()) {
    throw new Error(`${command}: no native identity backend in web mode`);
  }
}

export async function getIdentity(): Promise<Identity> {
  requireTauri("get_identity");
  return fromRawIdentity(await invokeTauri<RawIdentity>("get_identity"));
}

export async function getNsec(): Promise<string> {
  requireTauri("get_nsec");
  return invokeTauri<string>("get_nsec");
}

export async function importIdentity(nsec: string): Promise<Identity> {
  requireTauri("import_identity");
  return fromRawIdentity(
    await invokeTauri<RawIdentity>("import_identity", { nsec }),
  );
}

export async function persistCurrentIdentity(): Promise<Identity> {
  requireTauri("persist_current_identity");
  return fromRawIdentity(
    await invokeTauri<RawIdentity>("persist_current_identity"),
  );
}

/**
 * Wipe all local Buzz state (keychain, App Support, WebKit, nest, OAuth cache,
 * CLI symlinks) and relaunch into first-run onboarding.
 *
 * The app restarts after this call completes. Callers should keep the pending
 * state until the process exits and only handle errors (e.g. display a toast).
 */
export async function signOut(): Promise<void> {
  requireTauri("sign_out");
  await invokeTauri("sign_out");
}
