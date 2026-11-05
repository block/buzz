import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getAgentSandboxStatus,
  openAgentSandboxProfile,
  resetAgentSandboxProfile,
  setAgentSandboxEnabled,
} from "@/shared/api/tauriAgentSandbox";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export const agentSandboxStatusQueryKey = ["agentSandboxStatus"] as const;

type Confirmation = "disable" | "reset" | null;

function errorMessage(error: unknown, fallback: string): string {
  if (typeof error === "string") return error;
  if (error instanceof Error && error.message) return error.message;
  return fallback;
}

function restartSummary(restarted: number, failed: number): string | null {
  if (failed > 0) {
    return `${failed} agent${failed === 1 ? "" : "s"} failed to restart — check the Agents tab.`;
  }
  if (restarted > 0) {
    return `Restarted ${restarted} agent${restarted === 1 ? "" : "s"}.`;
  }
  return null;
}

export function AgentSandboxSettingsCard() {
  const queryClient = useQueryClient();
  const statusQuery = useQuery({
    queryKey: agentSandboxStatusQueryKey,
    queryFn: getAgentSandboxStatus,
  });
  const [confirmation, setConfirmation] = React.useState<Confirmation>(null);
  const [isBusy, setIsBusy] = React.useState(false);
  const [actionError, setActionError] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);

  const status = statusQuery.data;
  if (!status?.supported) {
    return null;
  }

  async function run(action: () => Promise<string | null>, fallback: string) {
    setIsBusy(true);
    setActionError(null);
    setNotice(null);
    try {
      setNotice(await action());
    } catch (error) {
      setActionError(errorMessage(error, fallback));
    } finally {
      setIsBusy(false);
      await queryClient.invalidateQueries({
        queryKey: agentSandboxStatusQueryKey,
      });
    }
  }

  function setEnabled(enabled: boolean) {
    return run(async () => {
      const result = await setAgentSandboxEnabled(enabled);
      return restartSummary(
        result.restarted_count,
        result.failed_restart_count,
      );
    }, "Couldn't change the agent sandbox setting.");
  }

  function handleToggle(checked: boolean) {
    if (checked) {
      void setEnabled(true);
    } else {
      setConfirmation("disable");
    }
  }

  function handleConfirm() {
    const pending = confirmation;
    setConfirmation(null);
    if (pending === "disable") {
      void setEnabled(false);
    } else if (pending === "reset") {
      void run(async () => {
        await resetAgentSandboxProfile();
        return "Sandbox profile reset. Restart agents to apply it.";
      }, "Couldn't reset the sandbox profile.");
    }
  }

  return (
    <div className="min-w-0 space-y-3">
      <SettingsOptionGroup
        data-testid="settings-agent-sandbox"
        description="Limits what local agents can read and write on this computer."
        title="Agent sandbox"
      >
        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="agent-sandbox-switch"
            >
              Run agents in a sandbox
            </label>
            <p
              className="text-sm font-normal text-muted-foreground/70"
              data-settings-subcopy
            >
              Agents can use their workspace, their tools, and the folders in
              your sandbox profile. Changing this restarts running agents.
            </p>
          </div>
          <Switch
            checked={status.enabled}
            data-testid="agent-sandbox-toggle"
            disabled={isBusy}
            id="agent-sandbox-switch"
            onCheckedChange={handleToggle}
          />
        </SettingsOptionRow>
        <SettingsOptionRow>
          <div className="min-w-0">
            <p className="text-sm font-medium">Sandbox profile</p>
            <p
              className="text-sm font-normal text-muted-foreground/70"
              data-settings-subcopy
            >
              Give agents access to more folders by editing this file. Changes
              apply when agents restart.
            </p>
            {status.profile_path ? (
              <p
                className="mt-1 break-all font-mono text-xs text-muted-foreground"
                data-testid="agent-sandbox-profile-path"
              >
                {status.profile_path}
              </p>
            ) : null}
          </div>
          <div className="flex shrink-0 gap-2">
            <Button
              data-testid="agent-sandbox-edit-profile"
              disabled={isBusy}
              onClick={() =>
                void run(async () => {
                  await openAgentSandboxProfile();
                  return null;
                }, "Couldn't open the sandbox profile.")
              }
              size="sm"
              variant="outline"
            >
              Edit
            </Button>
            <Button
              data-testid="agent-sandbox-reset-profile"
              disabled={isBusy}
              onClick={() => setConfirmation("reset")}
              size="sm"
              variant="outline"
            >
              Reset
            </Button>
          </div>
        </SettingsOptionRow>
      </SettingsOptionGroup>

      {status.enabled && status.problem ? (
        <p
          className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          data-testid="agent-sandbox-problem"
          role="alert"
        >
          Agents can't start in the sandbox: {status.problem}
        </p>
      ) : null}
      {actionError ? (
        <p
          className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          data-testid="agent-sandbox-error"
          role="alert"
        >
          {actionError}
        </p>
      ) : null}
      {notice ? (
        <p
          className="text-sm text-muted-foreground"
          data-testid="agent-sandbox-notice"
          role="status"
        >
          {notice}
        </p>
      ) : null}

      <AlertDialog
        onOpenChange={(open) => {
          if (!open) setConfirmation(null);
        }}
        open={confirmation !== null}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirmation === "reset"
                ? "Reset the sandbox profile?"
                : "Run agents without a sandbox?"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirmation === "reset"
                ? "Your edits to the sandbox profile will be replaced with the default."
                : "Agents will be able to read and change any file your account can, including keys and credentials. Running agents restart."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              data-testid="agent-sandbox-confirm"
              onClick={handleConfirm}
            >
              {confirmation === "reset" ? "Reset profile" : "Turn off sandbox"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
