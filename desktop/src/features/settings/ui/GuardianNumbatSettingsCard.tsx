import * as React from "react";

import {
  deactivateGuardianNumbat,
  getGuardianNumbatStatus,
  installGuardianNumbat,
  rollbackGuardianNumbat,
  type GuardianNumbatStatus,
  uninstallGuardianNumbat,
} from "@/shared/api/tauriNumbat";
import { Button } from "@/shared/ui/button";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

type LifecycleAction = "install" | "deactivate" | "rollback" | "uninstall";

const actionLabels: Record<LifecycleAction, string> = {
  install: "Install verified Numbat",
  deactivate: "Deactivate",
  rollback: "Roll back",
  uninstall: "Uninstall",
};

export function GuardianNumbatSettingsCard() {
  const [status, setStatus] = React.useState<GuardianNumbatStatus | null>(null);
  const [busy, setBusy] = React.useState<LifecycleAction | "loading" | null>(
    "loading",
  );
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    setBusy("loading");
    setError(null);
    try {
      setStatus(await getGuardianNumbatStatus());
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Guardian status is unavailable.",
      );
    } finally {
      setBusy(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (
    action: LifecycleAction,
    operation: () => Promise<GuardianNumbatStatus>,
  ) => {
    if (
      (action === "rollback" || action === "uninstall") &&
      !window.confirm(
        action === "rollback"
          ? "Roll Guardian Numbat back to the previous verified version? Buzz will repair its agent hooks as part of the change."
          : "Uninstall Buzz-managed Numbat? Buzz will remove its agent hooks before deleting the active version.",
      )
    ) {
      return;
    }
    setBusy(action);
    setError(null);
    try {
      setStatus(await operation());
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : `Guardian could not ${action} Numbat.`,
      );
      setStatus(await getGuardianNumbatStatus().catch(() => status));
    } finally {
      setBusy(null);
    }
  };

  const active = status?.state === "active";
  const stateLabel = status
    ? status.state.replaceAll("_", " ")
    : busy === "loading"
      ? "checking"
      : "unknown";

  return (
    <section className="min-w-0" data-testid="settings-guardian-numbat">
      <SettingsSectionHeader
        description="Buzz-managed endpoint visibility for local coding agents. Every launch rechecks the pinned component receipt and binary."
        title="Guardian Numbat"
      />
      <SettingsOptionGroup>
        <SettingsOptionRow className="items-start">
          <div className="min-w-0 space-y-1">
            <div className="font-medium capitalize">{stateLabel}</div>
            <div className="text-xs text-muted-foreground">
              {status?.detail ?? "Checking the managed component store…"}
            </div>
            {status?.version ? (
              <div className="text-xs text-muted-foreground">
                Version {status.version} · {status.target}
                {status.digestSuffix ? ` · SHA-256 …${status.digestSuffix}` : ""}
              </div>
            ) : null}
          </div>
          <Button disabled={busy !== null} onClick={() => void refresh()} variant="outline">
            Refresh
          </Button>
        </SettingsOptionRow>
        <SettingsOptionRow className="flex-wrap justify-end border-t border-border/50">
          {!active ? (
            <Button
              disabled={busy !== null}
              onClick={() => void run("install", installGuardianNumbat)}
            >
              {busy === "install" ? "Installing…" : actionLabels.install}
            </Button>
          ) : (
            <>
              <Button
                disabled={busy !== null || !status.rollbackAvailable}
                onClick={() => void run("rollback", rollbackGuardianNumbat)}
                variant="outline"
              >
                {busy === "rollback" ? "Rolling back…" : actionLabels.rollback}
              </Button>
              <Button
                disabled={busy !== null}
                onClick={() => void run("deactivate", deactivateGuardianNumbat)}
                variant="outline"
              >
                {busy === "deactivate" ? "Deactivating…" : actionLabels.deactivate}
              </Button>
              <Button
                disabled={busy !== null}
                onClick={() => void run("uninstall", uninstallGuardianNumbat)}
                variant="destructive"
              >
                {busy === "uninstall" ? "Uninstalling…" : actionLabels.uninstall}
              </Button>
            </>
          )}
        </SettingsOptionRow>
      </SettingsOptionGroup>
      {error ? (
        <p className="mt-3 text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
