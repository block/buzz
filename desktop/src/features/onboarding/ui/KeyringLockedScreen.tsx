import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { relaunch } from "@tauri-apps/plugin-process";
import { importIdentity } from "@/shared/api/tauriIdentity";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { NostrKeyImportForm } from "./NostrKeyImportForm";

export function KeyringLockedScreen() {
  const queryClient = useQueryClient();
  const systemColorScheme = useSystemColorScheme();
  const [showImport, setShowImport] = React.useState(false);

  const handleReimportClick = React.useCallback(() => {
    const confirmed = window.confirm(
      "Importing a different nsec replaces the identity currently held by the local secret provider for this install. The previous identity will no longer be accessible. Continue?",
    );
    if (confirmed) {
      setShowImport(true);
    }
  }, []);

  const handleImport = React.useCallback(
    async (nsec: string, password?: string) => {
      const identity = await importIdentity(nsec, password);
      // Update the identity query cache so useIdentityQuery observers see
      // locked: false. The bootedLocked latch in hooks.ts will then route
      // to RelaunchRequiredScreen via bootedLocked && !identityLocked.
      queryClient.setQueryData(["identity"], identity);
    },
    [queryClient],
  );

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-system-color-scheme={systemColorScheme}
      data-testid="keyring-locked"
    >
      <StartupWindowDragRegion />
      <div className="relative flex w-full max-w-[500px] flex-col items-center text-center">
        <h1 className="text-3xl font-semibold tracking-tight">
          Secret provider unavailable
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          Buzz could not reach the machine-local secret provider. Your identity
          was not replaced or copied to a file. Relaunch Buzz after the provider
          is restored, or re-import your key to replace it deliberately.
        </p>

        {showImport ? (
          <NostrKeyImportForm
            backLabel="Cancel"
            onBack={() => setShowImport(false)}
            onImport={handleImport}
          />
        ) : (
          <div className="mt-8 flex w-full max-w-[300px] flex-col gap-3">
            <Button
              className="h-10 w-full"
              data-testid="relaunch-app"
              onClick={() => {
                void relaunch();
              }}
              type="button"
            >
              Relaunch Buzz
            </Button>
            <Button
              className="h-10 w-full"
              onClick={handleReimportClick}
              type="button"
              variant="secondary"
            >
              Re-import your key instead
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
