import { invokeTauri } from "@/shared/api/tauri";
import type { Identity, IdentityStorage } from "@/shared/api/types";

type RawIdentity = {
  pubkey: string;
  display_name: string;
  storage?: IdentityStorage;
  lost?: boolean;
  locked?: boolean;
  reset_failed?: boolean;
};

function fromRawIdentity(raw: RawIdentity): Identity {
  return {
    pubkey: raw.pubkey,
    displayName: raw.display_name,
    storage: raw.storage,
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

export async function importIdentity(
  nsec: string,
  password?: string,
  recoverySecret?: string,
): Promise<Identity> {
  return fromRawIdentity(
    await invokeTauri<RawIdentity>("import_identity", {
      nsec,
      password,
      recoverySecret,
    }),
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
 * The app restarts after this call completes. Callers should keep the pending
 * state until the process exits and only handle errors (e.g. display a toast).
 */
export async function signOut(): Promise<void> {
  await invokeTauri("sign_out");
}

export type GeneratePassphraseOptions = {
  /** Word count; Rust clamps to its allowed range (currently 3–10). */
  words?: number;
  /** Separator joined between words. Defaults to a space in Rust. */
  separator?: string;
};

/** Generate a word passphrase (EFF short wordlist, OS entropy) in Rust. */
export async function generateBackupPassphrase(
  options?: GeneratePassphraseOptions,
): Promise<string> {
  return invokeTauri<string>("generate_backup_passphrase", {
    words: options?.words,
    separator: options?.separator,
  });
}

/** Encrypt the current identity as an in-memory NIP-49 backup for native save. */
export async function createNcryptsecBackup(password: string): Promise<string> {
  return invokeTauri<string>("create_ncryptsec_backup", { password });
}

export type CreatedTwoSkdBackup = {
  backup: string;
  recoverySecret: string;
};

/** Create a 2SKD backup and its separately-held recovery code in Rust. */
export async function createTwoSkdBackup(
  password: string,
): Promise<CreatedTwoSkdBackup> {
  return invokeTauri<CreatedTwoSkdBackup>("create_2skd_backup", { password });
}

/** Save only the encrypted half of a 2SKD recovery kit. */
export async function saveTwoSkdBackupCopy(
  backup: string,
): Promise<string | null> {
  return (
    (await invokeTauri<string | null>("save_2skd_backup_copy", { backup })) ??
    null
  );
}

/** Save the recovery code and public identity as a printable PDF. */
export async function saveTwoSkdRecoverySheet(
  backup: string,
  recoverySecret: string,
): Promise<string | null> {
  return (
    (await invokeTauri<string | null>("save_2skd_recovery_sheet", {
      backup,
      recoverySecret,
    })) ?? null
  );
}

/** Save a portable backup copy. Returns null when the native dialog is cancelled. */
export async function saveNcryptsecCopy(
  ncryptsec: string,
): Promise<string | null> {
  return (
    (await invokeTauri<string | null>("save_ncryptsec_copy", { ncryptsec })) ??
    null
  );
}

export type BackupVerification = {
  pubkey: string;
  npub: string;
  matchesCurrentIdentity: boolean;
};

/** Decrypt locally and return only the backup's public identity and match state. */
export async function verifyNcryptsecBackup(
  ncryptsec: string,
  password: string,
): Promise<BackupVerification> {
  return invokeTauri<BackupVerification>("verify_ncryptsec_backup", {
    ncryptsec,
    password,
  });
}

/** Verify all three inputs for a 2SKD recovery without exposing the nsec. */
export async function verifyTwoSkdBackup(
  backup: string,
  password: string,
  recoverySecret: string,
): Promise<BackupVerification> {
  return invokeTauri<BackupVerification>("verify_2skd_backup", {
    backup,
    password,
    recoverySecret,
  });
}
