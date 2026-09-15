import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

type CommunityApplyErrorScreenProps = {
  error: string;
  onChangeCommunity: () => void;
  onRetry: () => void;
};

export function CommunityApplyErrorScreen({
  error,
  onChangeCommunity,
  onRetry,
}: CommunityApplyErrorScreenProps) {
  const systemColorScheme = useSystemColorScheme();
  const [enterprise, setEnterprise] = useState(false);
  const [busy, setBusy] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    void invoke<boolean>("federated_identity_required")
      .then((required) => {
        if (active) setEnterprise(required);
      })
      .catch(() => {
        if (active)
          setLoginError("Could not read enterprise sign-in configuration.");
      });
    return () => {
      active = false;
    };
  }, []);
  async function signIn() {
    setBusy(true);
    setLoginError(null);
    try {
      await invoke("start_builderlab_login");
      onRetry();
    } catch (error) {
      setLoginError(String(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-system-color-scheme={systemColorScheme}
      data-testid="community-apply-error"
    >
      <StartupWindowDragRegion />
      <div className="relative flex w-full max-w-[500px] flex-col items-center text-center">
        <h1 className="text-3xl font-semibold tracking-tight">
          Community connection failed
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">{error}</p>
        {loginError ? (
          <p role="alert" className="mt-3 text-sm text-destructive">
            {loginError}
          </p>
        ) : null}
        <div className="mt-8 flex w-full max-w-[300px] flex-col gap-3">
          {enterprise ? (
            <Button
              onClick={signIn}
              disabled={busy}
              type="button"
              data-testid="enterprise-sign-in"
            >
              {busy
                ? "Waiting for browser sign-in…"
                : "Sign in with your work account"}
            </Button>
          ) : null}
          {busy ? (
            <Button
              variant="secondary"
              onClick={() => {
                void invoke("cancel_builderlab_login").catch((error) =>
                  setLoginError(String(error)),
                );
              }}
            >
              Cancel sign-in
            </Button>
          ) : null}
          <Button
            className="h-10 w-full"
            data-testid="community-apply-error-retry"
            onClick={onRetry}
            type="button"
          >
            Retry
          </Button>
          <Button
            className="h-10 w-full"
            onClick={onChangeCommunity}
            type="button"
            variant="secondary"
          >
            Change community
          </Button>
        </div>
      </div>
    </div>
  );
}
