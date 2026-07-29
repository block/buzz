/**
 * Pure state model for the encrypted-key-backup (NIP-49) creation flow,
 * shared by the onboarding DownloadKeyStep and the settings Password Backup
 * row.
 *
 * All validation and phase logic lives here so it can be unit-tested without
 * React. Hosts wire the reducer to the Tauri commands
 * (`generate_backup_passphrase`, `create_ncryptsec_backup`) and dispatch
 * events; the model never touches the raw private key — by construction the
 * default backup path cannot invoke `get_nsec`.
 *
 * Encryption is eager: the moment the passphrase is valid the host kicks off
 * the KDF in the background (`pendingEncryptPassphrase` tells it what to
 * start). Clicking Download either commits an already-finished result
 * immediately or flags `downloadPending` so the commit happens as soon as the
 * in-flight encryption lands. Results are keyed by passphrase, so stale
 * completions for an edited passphrase are ignored.
 *
 * The flow is a single password field. The "Generate password" popover is
 * host-local UI state (word count, separator, candidate) — accepting a
 * candidate simply dispatches `set-passphrase`, so the model doesn't
 * distinguish typed from generated passwords.
 */

/** Mirrors `MIN_PASSPHRASE_LEN` in `src-tauri/src/key_backup.rs`. */
export const MIN_PASSPHRASE_LEN = 12;

export type EncryptedBackupState = {
  passphrase: string;
  /** Passphrase whose KDF is currently running in the background, if any. */
  encryptingPassphrase: string | null;
  /** Most recent completed encryption, keyed by the passphrase it used. */
  encrypted: { passphrase: string; ncryptsec: string } | null;
  createError: string | null;
  /** True when Download was clicked while encryption was still in flight. */
  downloadPending: boolean;
  /** The committed `ncryptsec1…` blob once the user downloads the backup. */
  ncryptsec: string | null;
};

export const initialEncryptedBackupState: EncryptedBackupState = {
  passphrase: "",
  encryptingPassphrase: null,
  encrypted: null,
  createError: null,
  downloadPending: false,
  ncryptsec: null,
};

export type EncryptedBackupEvent =
  | { type: "set-passphrase"; value: string }
  | { type: "encrypt-started"; passphrase: string }
  | { type: "encrypt-succeeded"; passphrase: string; ncryptsec: string }
  | { type: "encrypt-failed"; passphrase: string; message: string }
  | { type: "download-clicked" };

export function encryptedBackupReducer(
  state: EncryptedBackupState,
  event: EncryptedBackupEvent,
): EncryptedBackupState {
  switch (event.type) {
    case "set-passphrase":
      return { ...state, passphrase: event.value, createError: null };
    case "encrypt-started":
      return {
        ...state,
        encryptingPassphrase: event.passphrase,
        createError: null,
      };
    case "encrypt-succeeded": {
      const next: EncryptedBackupState = {
        ...state,
        encryptingPassphrase:
          state.encryptingPassphrase === event.passphrase
            ? null
            : state.encryptingPassphrase,
        encrypted: {
          passphrase: event.passphrase,
          ncryptsec: event.ncryptsec,
        },
      };
      // A pending download commits the moment the matching result lands;
      // results for an edited (stale) passphrase never commit.
      if (state.downloadPending && event.passphrase === state.passphrase) {
        return {
          ...next,
          downloadPending: false,
          ncryptsec: event.ncryptsec,
        };
      }
      return next;
    }
    case "encrypt-failed": {
      const next: EncryptedBackupState = {
        ...state,
        encryptingPassphrase:
          state.encryptingPassphrase === event.passphrase
            ? null
            : state.encryptingPassphrase,
      };
      // Stale failures (passphrase already edited) are silent — a fresh
      // encryption for the current passphrase is already on its way.
      if (event.passphrase !== state.passphrase) return next;
      return { ...next, createError: event.message, downloadPending: false };
    }
    case "download-clicked": {
      const passphrase = effectivePassphrase(state);
      if (!passphrase || state.downloadPending || state.ncryptsec) return state;
      if (state.encrypted?.passphrase === passphrase) {
        return { ...state, ncryptsec: state.encrypted.ncryptsec };
      }
      return { ...state, downloadPending: true };
    }
  }
}

/**
 * Validation issue for the passphrase, or null when acceptable. An empty
 * field reports nothing so the user isn't scolded before typing.
 */
export function passphraseIssue(passphrase: string): string | null {
  if (passphrase.length === 0) return null;
  if ([...passphrase].length < MIN_PASSPHRASE_LEN) {
    return `Use at least ${MIN_PASSPHRASE_LEN} characters.`;
  }
  return null;
}

/** The passphrase a download would use, or null when not yet valid. */
export function effectivePassphrase(
  state: EncryptedBackupState,
): string | null {
  if ([...state.passphrase].length < MIN_PASSPHRASE_LEN) return null;
  return state.passphrase;
}

/**
 * The passphrase the host should start encrypting now, or null when nothing
 * needs to start (invalid, already encrypted, already in flight, or done).
 */
export function pendingEncryptPassphrase(
  state: EncryptedBackupState,
): string | null {
  if (state.ncryptsec) return null;
  const passphrase = effectivePassphrase(state);
  if (!passphrase) return null;
  if (state.encrypted?.passphrase === passphrase) return null;
  if (state.encryptingPassphrase === passphrase) return null;
  return passphrase;
}

/** Whether the current passphrase's encryption is still running. */
export function isEncrypting(state: EncryptedBackupState): boolean {
  const passphrase = effectivePassphrase(state);
  return passphrase !== null && state.encryptingPassphrase === passphrase;
}

/** Whether the Download action is currently actionable. */
export function downloadDisabled(state: EncryptedBackupState): boolean {
  return state.downloadPending || effectivePassphrase(state) === null;
}
