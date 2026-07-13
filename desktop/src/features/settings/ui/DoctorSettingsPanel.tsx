import * as React from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Circle,
  Download,
  ExternalLink,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  useAcpRuntimesQuery,
  useInstallAcpRuntimeMutation,
  useGitBashPrerequisiteQuery,
  useNodeRuntimeCheckQuery,
} from "@/features/agents/hooks";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type {
  AcpRuntimeCatalogEntry,
  AuthStatus,
  NodeRuntimeCheck,
} from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { SettingsOptionGroup } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

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

function AuthStatusBadge({ authStatus }: { authStatus: AuthStatus }) {
  switch (authStatus.status) {
    case "logged_in":
      return (
        <span className="inline-flex items-center gap-1 text-xs text-status-added">
          <CheckCircle2 className="h-3 w-3" />
          Authenticated
        </span>
      );
    case "logged_out":
      return (
        <span className="inline-flex items-center gap-1 text-xs text-warning">
          <AlertTriangle className="h-3 w-3" />
          Not authenticated
        </span>
      );
    case "config_invalid":
      return (
        <span className="inline-flex items-center gap-1 text-xs text-destructive">
          <XCircle className="h-3 w-3" />
          Config error
        </span>
      );
    case "not_applicable":
    case "unknown":
      return null;
  }
}

function InstallActions({
  hasError,
  isInstalling,
  onInstall,
  runtime,
}: {
  hasError: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const showInstall = runtime.canAutoInstall && !runtime.nodeRequired;

  return (
    <div className="mt-2 flex items-center gap-2">
      {showInstall ? (
        <Button
          disabled={isInstalling}
          onClick={onInstall}
          size="sm"
          type="button"
          variant="outline"
        >
          {isInstalling ? (
            <RefreshCw className="h-4 w-4 animate-spin" />
          ) : hasError ? (
            <RefreshCw className="h-4 w-4" />
          ) : (
            <Download className="h-4 w-4" />
          )}
          {isInstalling ? "Installing..." : hasError ? "Retry" : "Install"}
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

/**
 * Node.js callout when required, or the install actions when it is not.
 * Used for both `adapter_missing` and `not_installed` availability states.
 * The `cli_missing` branch is intentionally excluded — its install path does
 * not involve npm, so no Node.js gate applies.
 */
function NodeRequiredOrInstall({
  hasError,
  isInstalling,
  onInstall,
  runtime,
}: {
  hasError: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  if (runtime.nodeRequired) {
    return (
      <p className="mt-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-sm text-amber-700 dark:text-amber-400">
        Node.js is required to install this adapter.{" "}
        <button
          className="underline underline-offset-2 hover:no-underline"
          onClick={() => void openUrl("https://nodejs.org")}
          type="button"
        >
          Install Node.js
        </button>
        , then click Re-run.
      </p>
    );
  }
  return (
    <InstallActions
      hasError={hasError}
      isInstalling={isInstalling}
      onInstall={onInstall}
      runtime={runtime}
    />
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
            {/*
             * Auth badge renders only for `available` runtimes: non-available
             * entries always have auth_status: unknown (no probe was run), which
             * AuthStatusBadge maps to null. Rendering it here is self-consistent.
             */}
            {runtime.authStatus.status !== "not_applicable" &&
            runtime.authStatus.status !== "unknown" ? (
              <div className="mt-2">
                <AuthStatusBadge authStatus={runtime.authStatus} />
              </div>
            ) : null}
            {/* Login hint shown when not logged in or the config is invalid */}
            {runtime.loginHint &&
            runtime.authStatus.status !== "not_applicable" &&
            runtime.authStatus.status !== "unknown" ? (
              <p className="mt-1 text-sm text-muted-foreground">
                {runtime.authStatus.status === "config_invalid"
                  ? `Config error: ${runtime.authStatus.diagnostic}`
                  : runtime.loginHint}
              </p>
            ) : null}
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
            <NodeRequiredOrInstall
              hasError={installError !== null}
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
              hasError={installError !== null}
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
            <NodeRequiredOrInstall
              hasError={installError !== null}
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
          <p className="mt-2 whitespace-pre-line rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive">
            {installError}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function GitBashRow({
  prerequisite,
}: {
  prerequisite: NonNullable<
    ReturnType<typeof useGitBashPrerequisiteQuery>["data"]
  >;
}) {
  return (
    <div
      className="flex min-h-16 items-start gap-3 bg-amber-500/5 px-4 py-3 text-sm"
      data-testid="doctor-git-bash"
    >
      <div className="mt-0.5 shrink-0">
        {prerequisite.available ? (
          <CheckCircle2 className="h-4 w-4 text-status-added" />
        ) : (
          <AlertTriangle className="h-4 w-4 text-warning" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">Git Bash</p>
        {prerequisite.available ? (
          <p className="mt-1 break-all font-mono text-2xs text-muted-foreground/80">
            {prerequisite.path}
          </p>
        ) : (
          <>
            <p className="mt-1 text-sm text-muted-foreground">
              Required for buzz-agent shell tools on Windows.
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {prerequisite.installHint}
            </p>
            <button
              className="mt-2 inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => void openUrl(prerequisite.installInstructionsUrl)}
              type="button"
            >
              <ExternalLink className="h-4 w-4" /> Install Git for Windows
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/**
 * Node.js runtime requirement of the bundled ACP bridges (shell shims that
 * `exec node`). Rendered only when the app ships npm-sourced bridges — the
 * check query returns null otherwise.
 */
function NodeRuntimeSection({ check }: { check: NodeRuntimeCheck }) {
  return (
    <div
      className={cn(
        "flex min-h-16 items-start gap-3 px-4 py-3 text-sm",
        check.status === "pass" ? "bg-background/60" : "bg-amber-500/5",
      )}
      data-testid="doctor-node-runtime"
    >
      <div className="mt-0.5 shrink-0">
        {check.status === "pass" ? (
          <CheckCircle2 className="h-4 w-4 text-status-added" />
        ) : (
          <AlertTriangle className="h-4 w-4 text-warning" />
        )}
      </div>

      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">Node.js runtime</p>
        <p className="mt-1 text-sm font-normal text-muted-foreground">
          {check.message}.
        </p>
        {check.nodePath ? (
          <p className="mt-1 break-all font-mono text-2xs text-muted-foreground/80">
            {check.nodePath}
          </p>
        ) : null}
        {check.requirements.length > 0 ? (
          <ul className="mt-2 space-y-0.5">
            {check.requirements.map((req) => (
              <li
                className="text-2xs text-muted-foreground/80"
                key={req.binary}
              >
                <code className="rounded bg-muted px-1 py-0.5">
                  {req.binary}
                </code>{" "}
                requires Node.js {req.requirement} —{" "}
                {req.verdict === "satisfied"
                  ? "satisfied"
                  : req.verdict === "unmet"
                    ? "unmet"
                    : "unknown"}
              </li>
            ))}
          </ul>
        ) : null}
        {check.status === "warn" ? (
          <button
            className="mt-2 inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            onClick={() => void openUrl(check.fixUrl)}
            type="button"
          >
            <ExternalLink className="h-4 w-4" />
            Install Node.js
          </button>
        ) : null}
      </div>
    </div>
  );
}

export function DoctorSettingsPanel() {
  const runtimesQuery = useAcpRuntimesQuery();
  const gitBashQuery = useGitBashPrerequisiteQuery();
  const runtimes = runtimesQuery.data ?? [];
  const nodeRuntimeQuery = useNodeRuntimeCheckQuery();
  const isRefreshing = runtimesQuery.isFetching || nodeRuntimeQuery.isFetching;
  const installMutation = useInstallAcpRuntimeMutation();
  const [installResults, setInstallResults] = React.useState<
    Record<string, { success: boolean; error: string | null }>
  >({});
  // Per-runtime installing state: tracks which runtime IDs have an in-flight
  // install so concurrent installs each show their own spinner correctly.
  const [installingIds, setInstallingIds] = React.useState<Set<string>>(
    new Set(),
  );

  function handleInstall(runtimeId: string) {
    // Clear any previous result for this runtime before retrying.
    setInstallResults((prev) => ({
      ...prev,
      [runtimeId]: { success: false, error: null },
    }));
    setInstallingIds((prev) => new Set(prev).add(runtimeId));

    installMutation.mutate(runtimeId, {
      onSuccess: (result) => {
        if (result.success) {
          setInstallResults((prev) => ({
            ...prev,
            [runtimeId]: { success: true, error: null },
          }));
        } else {
          setInstallResults((prev) => ({
            ...prev,
            [runtimeId]: {
              success: false,
              error: getInstallErrorMessage(result.steps),
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
      onSettled: () => {
        setInstallingIds((prev) => {
          const next = new Set(prev);
          next.delete(runtimeId);
          return next;
        });
      },
    });
  }

  return (
    <section className="min-w-0" data-testid="settings-doctor">
      <SettingsSectionHeader
        title="Doctor"
        description="Verify the ACP runtime commands available to the desktop app."
        action={
          <Button
            disabled={isRefreshing}
            onClick={() => {
              setInstallResults({});
              void runtimesQuery.refetch();
              void gitBashQuery.refetch();
              void nodeRuntimeQuery.refetch();
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
        }
      />

      <div className="space-y-5">
        <SettingsOptionGroup>
          {gitBashQuery.data ? (
            <>
              <div className="px-4 py-3 text-sm">
                <h3 className="text-sm font-medium">System prerequisites</h3>
                <p className="mt-1 text-sm font-normal text-muted-foreground">
                  Windows tools required by supported agents.
                </p>
              </div>
              <GitBashRow prerequisite={gitBashQuery.data} />
            </>
          ) : null}
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
                isInstalling={installingIds.has(runtime.id)}
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

          {nodeRuntimeQuery.data ? (
            <NodeRuntimeSection check={nodeRuntimeQuery.data} />
          ) : null}

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
