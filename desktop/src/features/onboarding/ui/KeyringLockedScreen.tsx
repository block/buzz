import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { relaunch } from "@tauri-apps/plugin-process";
import { useTranslation } from "@/i18n";
import { importIdentity } from "@/shared/api/tauriIdentity";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { NostrKeyImportForm } from "./NostrKeyImportForm";

export function KeyringLockedScreen() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const systemColorScheme = useSystemColorScheme();
  const [showImport, setShowImport] = React.useState(false);

  const handleReimportClick = React.useCallback(() => {
    const confirmed = window.confirm(t("onboarding.recovery.keyring-confirm"));
    if (confirmed) {
      setShowImport(true);
    }
  }, [t]);

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
          {t("onboarding.recovery.keyring-title")}
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          {t("onboarding.recovery.keyring-body")}
        </p>

        {showImport ? (
          <NostrKeyImportForm
            backLabel={t("onboarding.recovery.keyring-cancel")}
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
              {t("onboarding.recovery.relaunch")}
            </Button>
            <Button
              className="h-10 w-full"
              onClick={handleReimportClick}
              type="button"
              variant="secondary"
            >
              {t("onboarding.recovery.keyring-reimport")}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
