/** Pure state model for NIP-49 backup creation. */
export const MIN_PASSPHRASE_LEN = 12;

export type EncryptedBackupState = {
  passphrase: string;
  requestId: number | null;
  nextRequestId: number;
  encrypted: string | null;
  createError: string | null;
  downloadPending: boolean;
  ncryptsec: string | null;
  savedPassword: boolean;
};

export const initialEncryptedBackupState: EncryptedBackupState = {
  passphrase: "",
  requestId: null,
  nextRequestId: 1,
  encrypted: null,
  createError: null,
  downloadPending: false,
  ncryptsec: null,
  savedPassword: false,
};

export type EncryptedBackupEvent =
  | { type: "set-passphrase"; value: string }
  | { type: "encrypt-started"; requestId: number }
  | { type: "encrypt-succeeded"; requestId: number; ncryptsec: string }
  | { type: "encrypt-failed"; requestId: number; message: string }
  | { type: "download-clicked" }
  | { type: "back-to-password" }
  | { type: "start-new-backup" };

export function encryptedBackupReducer(
  state: EncryptedBackupState,
  event: EncryptedBackupEvent,
): EncryptedBackupState {
  switch (event.type) {
    case "set-passphrase":
      return {
        ...state,
        passphrase: event.value,
        encrypted: null,
        createError: null,
      };
    case "encrypt-started":
      return {
        ...state,
        requestId: event.requestId,
        nextRequestId: Math.max(state.nextRequestId, event.requestId + 1),
        createError: null,
      };
    case "encrypt-succeeded":
      if (event.requestId !== state.requestId) return state;
      if (state.downloadPending) {
        return {
          ...state,
          passphrase: "",
          requestId: null,
          encrypted: event.ncryptsec,
          ncryptsec: event.ncryptsec,
          downloadPending: false,
          savedPassword: true,
        };
      }
      return {
        ...state,
        requestId: null,
        encrypted: event.ncryptsec,
      };
    case "encrypt-failed":
      if (event.requestId !== state.requestId) return state;
      return {
        ...state,
        passphrase: "",
        requestId: null,
        createError: event.message,
        downloadPending: false,
      };
    case "download-clicked":
      if (
        state.ncryptsec ||
        state.downloadPending ||
        (!state.encrypted && !effectivePassphrase(state))
      )
        return state;
      return state.encrypted
        ? {
            ...state,
            ncryptsec: state.encrypted,
            passphrase: "",
            savedPassword: true,
          }
        : { ...state, downloadPending: true };
    case "back-to-password":
      return { ...state, createError: null };
    case "start-new-backup":
      return {
        ...initialEncryptedBackupState,
        nextRequestId: state.nextRequestId + 1,
      };
  }
}

export function passphraseIssue(passphrase: string): string | null {
  if (passphrase.length === 0) return null;
  return [...passphrase].length < MIN_PASSPHRASE_LEN
    ? `Use at least ${MIN_PASSPHRASE_LEN} characters.`
    : null;
}
export function effectivePassphrase(
  state: EncryptedBackupState,
): string | null {
  return [...state.passphrase].length < MIN_PASSPHRASE_LEN
    ? null
    : state.passphrase;
}
export function pendingEncryptPassphrase(
  state: EncryptedBackupState,
): string | null {
  if (state.savedPassword || state.encrypted || state.requestId !== null)
    return null;
  return effectivePassphrase(state);
}
export function isEncrypting(state: EncryptedBackupState): boolean {
  return state.requestId !== null;
}
export function downloadDisabled(state: EncryptedBackupState): boolean {
  if (state.savedPassword && state.ncryptsec) return false;
  return (
    state.downloadPending ||
    (!state.encrypted && effectivePassphrase(state) === null)
  );
}
