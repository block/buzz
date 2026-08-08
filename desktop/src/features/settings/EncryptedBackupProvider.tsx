import * as React from "react";
import { toast } from "sonner";

import {
  createNcryptsecBackup,
  saveNcryptsecCopy,
} from "@/shared/api/tauriIdentity";
import {
  type EncryptedBackupEvent,
  type EncryptedBackupState,
  encryptedBackupReducer,
  initialEncryptedBackupState,
  pendingEncryptPassphrase,
} from "./lib/encryptedBackup";

const ENCRYPT_DEBOUNCE_MS = 400;
/** How long a completed encrypted backup remains available in memory. */
export const BACKUP_AVAILABILITY_MS = 5 * 60 * 1000;
const BACKUP_READY_TOAST_ID = "encrypted-key-backup-ready";

type EncryptedBackupContextValue = {
  state: EncryptedBackupState;
  dispatch: React.Dispatch<EncryptedBackupEvent>;
  backupAvailable: boolean;
  availableUntil: number | null;
  isSaving: boolean;
  saveError: string | null;
  downloadBackup: () => Promise<void>;
  startNewBackup: () => void;
};

const EncryptedBackupContext =
  React.createContext<EncryptedBackupContextValue | null>(null);

export function EncryptedBackupProvider({
  children,
  onOpenSettings,
}: {
  children: React.ReactNode;
  onOpenSettings: () => void;
}) {
  const [state, dispatch] = React.useReducer(
    encryptedBackupReducer,
    initialEncryptedBackupState,
  );
  const [availableUntil, setAvailableUntil] = React.useState<number | null>(
    null,
  );
  const [isSaving, setIsSaving] = React.useState(false);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const autoSaveStartedForRef = React.useRef<string | null>(null);
  const mountedRef = React.useRef(true);
  const onOpenSettingsRef = React.useRef(onOpenSettings);

  React.useEffect(() => {
    onOpenSettingsRef.current = onOpenSettings;
  }, [onOpenSettings]);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  React.useEffect(() => {
    if (availableUntil === null) return;
    const expiresIn = Math.max(0, availableUntil - Date.now());
    const timer = window.setTimeout(() => {
      autoSaveStartedForRef.current = null;
      setAvailableUntil(null);
      setSaveError(null);
      dispatch({ type: "start-new-backup" });
      toast.dismiss(BACKUP_READY_TOAST_ID);
    }, expiresIn);
    return () => window.clearTimeout(timer);
  }, [availableUntil]);

  const pendingPassphrase = pendingEncryptPassphrase(state);
  const skipDebounce = state.downloadPending;
  React.useEffect(() => {
    if (!pendingPassphrase) return;
    let started = false;
    let cancelledBeforeStart = false;
    const requestId = state.nextRequestId;
    const start = () => {
      if (cancelledBeforeStart) return;
      started = true;
      dispatch({ type: "encrypt-started", requestId });
      void createNcryptsecBackup(pendingPassphrase)
        .then((ncryptsec) => {
          dispatch({ type: "encrypt-succeeded", requestId, ncryptsec });
        })
        .catch((err: unknown) => {
          dispatch({
            type: "encrypt-failed",
            requestId,
            message:
              err instanceof Error
                ? err.message
                : "Failed to encrypt your key.",
          });
        });
    };
    const timer = window.setTimeout(
      start,
      skipDebounce ? 0 : ENCRYPT_DEBOUNCE_MS,
    );
    return () => {
      if (!started) cancelledBeforeStart = true;
      window.clearTimeout(timer);
    };
  }, [pendingPassphrase, skipDebounce, state.nextRequestId]);

  React.useEffect(() => {
    if (state.downloadPending) {
      toast.loading("Preparing backup…", {
        description: "You can close this window while Buzz finishes.",
        duration: Number.POSITIVE_INFINITY,
        id: BACKUP_READY_TOAST_ID,
      });
      return;
    }
    if (
      state.createError &&
      state.passphrase.length === 0 &&
      !state.ncryptsec
    ) {
      toast.error("Couldn’t create backup", {
        description: state.createError,
        id: BACKUP_READY_TOAST_ID,
      });
    }
  }, [
    state.createError,
    state.downloadPending,
    state.ncryptsec,
    state.passphrase.length,
  ]);

  const showAvailableToast = React.useCallback(
    (description: string, error = false) => {
      const options = {
        action: {
          label: "Open settings",
          onClick: () => onOpenSettingsRef.current(),
        },
        description,
        id: BACKUP_READY_TOAST_ID,
      };
      if (error) toast.error("Backup ready to download", options);
      else toast.success("Backup ready to download", options);
    },
    [],
  );

  const saveBackup = React.useCallback(
    async (ncryptsec: string) => {
      if (isSaving) return;
      setIsSaving(true);
      setSaveError(null);
      toast("Saving backup…", {
        description: "The download window will open when it’s ready.",
        id: BACKUP_READY_TOAST_ID,
      });
      try {
        const path = await saveNcryptsecCopy(ncryptsec);
        if (mountedRef.current) {
          showAvailableToast(
            path === null
              ? "Your backup will be available to download for 5 minutes."
              : "You can download another copy for 5 minutes.",
          );
        }
      } catch (err) {
        if (!mountedRef.current) return;
        const message =
          err instanceof Error ? err.message : "Failed to save your key.";
        setSaveError(message);
        showAvailableToast(
          `${message} It will be available to download for 5 minutes.`,
          true,
        );
      } finally {
        if (mountedRef.current) setIsSaving(false);
      }
    },
    [isSaving, showAvailableToast],
  );

  React.useEffect(() => {
    const ncryptsec = state.ncryptsec;
    if (!ncryptsec || autoSaveStartedForRef.current === ncryptsec) return;
    autoSaveStartedForRef.current = ncryptsec;
    setAvailableUntil(Date.now() + BACKUP_AVAILABILITY_MS);
    void saveBackup(ncryptsec);
  }, [saveBackup, state.ncryptsec]);

  const downloadBackup = React.useCallback(async () => {
    if (!state.ncryptsec) return;
    await saveBackup(state.ncryptsec);
  }, [saveBackup, state.ncryptsec]);

  const startNewBackup = React.useCallback(() => {
    autoSaveStartedForRef.current = null;
    setAvailableUntil(null);
    setSaveError(null);
    toast.dismiss(BACKUP_READY_TOAST_ID);
    dispatch({ type: "start-new-backup" });
  }, []);

  const value = React.useMemo<EncryptedBackupContextValue>(
    () => ({
      state,
      dispatch,
      backupAvailable:
        state.savedPassword &&
        state.ncryptsec !== null &&
        availableUntil !== null,
      availableUntil,
      isSaving,
      saveError,
      downloadBackup,
      startNewBackup,
    }),
    [
      availableUntil,
      downloadBackup,
      isSaving,
      saveError,
      startNewBackup,
      state,
    ],
  );

  return (
    <EncryptedBackupContext.Provider value={value}>
      {children}
    </EncryptedBackupContext.Provider>
  );
}

export function useEncryptedBackup(): EncryptedBackupContextValue {
  const value = React.useContext(EncryptedBackupContext);
  if (!value) {
    throw new Error(
      "useEncryptedBackup must be used within EncryptedBackupProvider",
    );
  }
  return value;
}
