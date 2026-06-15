import * as React from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Circle,
  Download,
  ExternalLink,
  RefreshCw,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  useAcpRuntimesQuery,
  useInstallAcpRuntimeMutation,
} from "@/features/agents/hooks";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { SettingsOptionGroup } from "./SettingsOptionGroup";

function StatusIcon({
  availability,
}: {
  availability: AcpRuntimeCatalogEntry["availability"];
}) {
  switch (availability) {
    case "available":
      return <CheckCircle2 className="h-4 w-4 text-status-added" />;
    case "adapter_missing":
      return <AlertTriangle className="h-4 w-4 text-warning" />;
    case "cli_missing":
      return <AlertTriangle className="h-4 w-4 text-warning" />;
    case "not_installed":
      return <Circle className="h-4 w-4 text-muted-foreground/50" />;
  }
}

function InstallActions({
  isInstalling,
  onInstall,
  runtime,
}: {
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  return (
    <div className="mt-2 flex items-center gap-2">
      {runtime.canAutoInstall ? (
        <Button
          disabled={isInstalling}
          onClick={onInstall}
          size="sm"
          type="button"
          variant="outline"
        >
          {isInstalling ? (
            <RefreshCw className="h-4 w-4 animate-spin" />
          ) : (
            <Download className="h-4 w-4" />
          )}
          {isInstalling ? "Installing..." : "Install"}
        </Button>
      ) : null}
      <button
        className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
        onClick={() => void openUrl(runtime.installInstructionsUrl)}
        type="button"
      >
        <ExternalLink className="h-4 w-4" />
        View instructions
      </button>
    </div>
  );
}

function RuntimeRow({
  installError,
  installSuccess,
  isInstalling,
  onInstall,
  runtime,
}: {
  installError: string | null;
  installSuccess: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  return (
    <div
      className={cn(
        "flex min-h-16 items-start gap-3 px-4 py-3 text-sm",
        runtime.availability === "available"
          ? "bg-background/60"
          : runtime.availability === "adapter_missing" ||
              runtime.availability === "cli_missing"
            ? "bg-amber-500/5"
            : "bg-muted/20",
      )}
      data-testid={`doctor-runtime-${runtime.id}`}
    >
      <div className="mt-0.5 shrink-0">
        <StatusIcon availability={runtime.availability} />
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm font-medium">{runtime.label}</p>
          {runtime.command ? (
            <code className="rounded bg-muted px-1.5 py-0.5 text-2xs">
              {runtime.command}
            </code>
          ) : null}
        </div>

        {runtime.availability === "available" &&
        runtime.command &&
        runtime.binaryPath ? (
          <>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              Available via{" "}
              {describeResolvedCommand(runtime.command, runtime.binaryPath)}.
            </p>
            {runtime.defaultArgs.length > 0 ? (
              <p className="mt-2 text-xs text-muted-foreground">
                Default args:{" "}
                <code className="font-mono">
                  {runtime.defaultArgs.join(", ")}
                </code>
              </p>
            ) : null}
            {runtime.underlyingCliPath &&
            runtime.underlyingCliPath !== runtime.binaryPath ? (
              <div className="mt-1 space-y-0.5">
                <p className="break-all font-mono text-2xs text-muted-foreground/80">
                  <span className="text-muted-foreground">CLI:</span>{" "}
                  {runtime.underlyingCliPath}
                </p>
                <p className="break-all font-mono text-2xs text-muted-foreground/80">
                  <span className="text-muted-foreground">ACP adapter:</span>{" "}
                  {runtime.binaryPath}
                </p>
              </div>
            ) : (
              <>
                <p className="mt-1 break-all font-mono text-2xs text-muted-foreground/80">
                  {runtime.binaryPath}
                </p>
                <p className="mt-1 text-2xs text-muted-foreground/60">
                  ACP support built-in — no separate adapter needed.
                </p>
              </>
            )}
          </>
        ) : runtime.availability === "adapter_missing" ? (
          <>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              CLI detected at{" "}
              <code className="rounded bg-muted px-1 py-0.5 text-2xs">
                {runtime.underlyingCliPath ?? "unknown path"}
              </code>{" "}
              but ACP adapter not found.
            </p>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              {runtime.installHint}
            </p>
            <InstallActions
              isInstalling={isInstalling}
              onInstall={onInstall}
              runtime={runtime}
            />
          </>
        ) : runtime.availability === "cli_missing" ? (
          <>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              ACP adapter found at{" "}
              <code className="rounded bg-muted px-1 py-0.5 text-2xs">
                {runtime.binaryPath ?? "unknown path"}
              </code>{" "}
              but the {runtime.label} CLI is not installed.
            </p>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              {runtime.installHint}
            </p>
            <InstallActions
              isInstalling={isInstalling}
              onInstall={onInstall}
              runtime={runtime}
            />
          </>
        ) : (
          <>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              Not installed
            </p>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              {runtime.installHint}
            </p>
            <InstallActions
              isInstalling={isInstalling}
              onInstall={onInstall}
              runtime={runtime}
            />
          </>
        )}

        {installSuccess && runtime.availability !== "available" ? (
          <p className="mt-2 rounded-lg border border-green-500/30 bg-green-500/10 px-3 py-1.5 text-sm text-green-700 dark:text-green-400">
            Installed successfully!
          </p>
        ) : null}
        {installError ? (
          <p className="mt-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive">
            {installError}
          </p>
        ) : null}
      </div>
    </div>
  );
}

export function DoctorSettingsPanel() {
  const runtimesQuery = useAcpRuntimesQuery();
  const runtimes = runtimesQuery.data ?? [];
  const isRefreshing = runtimesQuery.isFetching;
  const installMutation = useInstallAcpRuntimeMutation();
  const [installResults, setInstallResults] = React.useState<
    Record<string, { success: boolean; error: string | null }>
  >({});

  function handleInstall(runtimeId: string) {
    setInstallResults((prev) => ({
      ...prev,
      [runtimeId]: { success: false, error: null },
    }));
    installMutation.mutate(runtimeId, {
      onSuccess: (result) => {
        if (result.success) {
          setInstallResults((prev) => ({
            ...prev,
            [runtimeId]: { success: true, error: null },
          }));
        } else {
          const lastStep = result.steps[result.steps.length - 1];
          setInstallResults((prev) => ({
            ...prev,
            [runtimeId]: {
              success: false,
              error: lastStep
                ? `Step "${lastStep.step}" failed: ${lastStep.stderr || lastStep.stdout || "unknown error"}`
                : "Install failed with no output.",
            },
          }));
        }
      },
      onError: (error) => {
        setInstallResults((prev) => ({
          ...prev,
          [runtimeId]: {
            success: false,
            error: error instanceof Error ? error.message : "Install failed.",
          },
        }));
      },
    });
  }

  return (
    <section data-testid="settings-doctor">
      <div className="mb-12 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <h2 className="text-2xl font-semibold tracking-tight">Doctor</h2>
          <p className="mt-1 text-base font-normal text-muted-foreground">
            Verify the ACP runtime commands available to the desktop app.
          </p>
        </div>

        <Button
          className="shrink-0"
          disabled={isRefreshing}
          onClick={() => {
            setInstallResults({});
            void runtimesQuery.refetch();
          }}
          size="sm"
          type="button"
          variant="outline"
        >
          <RefreshCw
            className={cn("h-4 w-4", isRefreshing && "animate-spin")}
          />
          Re-run
        </Button>
      </div>

      <div className="space-y-5">
        <SettingsOptionGroup>
          <div className="px-4 py-3 text-sm">
            <h3 className="text-sm font-medium">Agent CLIs and ACP runtimes</h3>
            <p className="mt-1 text-sm font-normal text-muted-foreground">
              Installation status of supported agent CLIs and their ACP
              runtimes.
            </p>
          </div>

          {runtimesQuery.isLoading ? (
            <div className="px-4 py-3 text-sm font-normal text-muted-foreground">
              Looking for ACP runtimes...
            </div>
          ) : runtimes.length > 0 ? (
            runtimes.map((runtime) => (
              <RuntimeRow
                installError={installResults[runtime.id]?.error ?? null}
                installSuccess={installResults[runtime.id]?.success ?? false}
                isInstalling={
                  installMutation.isPending &&
                  installMutation.variables === runtime.id
                }
                key={runtime.id}
                onInstall={() => handleInstall(runtime.id)}
                runtime={runtime}
              />
            ))
          ) : (
            <div className="bg-amber-500/10 px-4 py-3 text-sm text-warning">
              No known ACP runtimes found.
            </div>
          )}

          {runtimesQuery.error instanceof Error ? (
            <p className="bg-destructive/10 px-4 py-3 text-sm text-destructive">
              {runtimesQuery.error.message}
            </p>
          ) : null}
        </SettingsOptionGroup>
      </div>
    </section>
  );
}
