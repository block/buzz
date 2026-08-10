/**
 * Keeps the Rust-side active vault in sync with the stored path.
 *
 * The localStorage path is only a hint — access is granted by `set_active_vault`
 * on the Rust side, which holds the real root. On mount (and on every vault
 * change) we re-activate, because the backend forgets it on restart.
 */
import * as React from "react";
import { toast } from "sonner";

import {
  clearActiveVault,
  pickVaultFolder,
  setActiveVault,
} from "@/shared/api/vault";
import {
  storeVaultPath,
  useVaultPath,
} from "@/features/documents/useVaultPath";

export type VaultActivationState =
  | { status: "idle" }
  | { status: "activating" }
  | { status: "ready"; name: string; path: string }
  | { status: "error"; message: string };

export function useVaultLifecycle() {
  const vaultPath = useVaultPath();
  const [activation, setActivation] = React.useState<VaultActivationState>(
    () => (vaultPath ? { status: "activating" } : { status: "idle" }),
  );

  React.useEffect(() => {
    if (!vaultPath) {
      setActivation({ status: "idle" });
      void clearActiveVault().catch(() => {
        // Nothing to clear, or the backend is already down. Either way the UI
        // is already showing the empty state.
      });
      return;
    }

    let cancelled = false;
    setActivation({ status: "activating" });

    void setActiveVault(vaultPath)
      .then((info) => {
        if (cancelled) return;
        setActivation({
          name: info.name,
          path: info.path,
          status: "ready",
        });
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        const message =
          error instanceof Error
            ? error.message
            : "Could not open that folder.";
        setActivation({ message, status: "error" });
      });

    return () => {
      cancelled = true;
    };
  }, [vaultPath]);

  /** Show the folder picker and adopt the chosen folder as the vault. */
  const chooseVault = React.useCallback(async () => {
    try {
      const picked = await pickVaultFolder();
      if (!picked) return;
      // Activate before persisting so a rejected folder never gets stored.
      const info = await setActiveVault(picked);
      storeVaultPath(info.path);
    } catch (error: unknown) {
      const message =
        error instanceof Error ? error.message : "Could not open that folder.";
      toast.error(message);
    }
  }, []);

  const forgetVault = React.useCallback(() => {
    storeVaultPath(null);
  }, []);

  return { activation, chooseVault, forgetVault, vaultPath };
}
