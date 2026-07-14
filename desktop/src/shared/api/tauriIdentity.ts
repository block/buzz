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

export async function getIdentity(): Promise<Identity> {
  return fromRawIdentity(await invokeTauri<RawIdentity>("get_identity"));
}

export async function getNsec(): Promise<string> {
  return invokeTauri<string>("get_nsec");
}

export async function importIdentity(nsec: string): Promise<Identity> {
  return fromRawIdentity(
    await invokeTauri<RawIdentity>("import_identity", { nsec }),
  );
}

export async function persistCurrentIdentity(): Promise<Identity> {
  return fromRawIdentity(
    await invokeTauri<RawIdentity>("persist_current_identity"),
  );
}

/**
 * Wipe all local Buzz state (keychain, App Support, WebKit, nest, OAuth cache,
 * CLI symlinks) and relaunch into first-run onboarding.
 *
 * This call never resolves on success — the process restarts. Callers should
 * handle the error case (e.g. display a toast) but not await a resolution.
 */
export async function signOut(): Promise<void> {
  await invokeTauri("sign_out");
}
