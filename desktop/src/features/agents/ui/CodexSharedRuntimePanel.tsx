import * as React from "react";
import { CircleAlert, CircleCheck, MonitorUp, RefreshCw } from "lucide-react";
import { toast } from "sonner";

import {
  useCodexSharedRuntimeQuery,
  useLaunchCodexDesktopSharedMutation,
  useSetupCodexSharedRuntimeMutation,
  useTakeOverCodexDesktopSharedMutation,
} from "@/features/agents/codexSharedRuntimeHooks";
import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import {
  hasCodexDesktopRuntimeConflict,
  isCodexSharedRuntimeUsable,
} from "@/features/agents/codexSharedRuntimeStatus";
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

export function CodexSharedRuntimePanel({
  enabled = true,
}: {
  enabled?: boolean;
}) {
  const statusQuery = useCodexSharedRuntimeQuery({ enabled });
  const runtimesQuery = useAcpRuntimesQuery({ enabled });
  const setupMutation = useSetupCodexSharedRuntimeMutation();
  const launchMutation = useLaunchCodexDesktopSharedMutation();
  const takeoverMutation = useTakeOverCodexDesktopSharedMutation();
  const [confirmTakeover, setConfirmTakeover] = React.useState(false);
  const [setupError, setSetupError] = React.useState<string | null>(null);
  const status = statusQuery.data;
  const ready = status?.state === "ready";
  const conflict = hasCodexDesktopRuntimeConflict(status);
  const usable = isCodexSharedRuntimeUsable(status);
  const codexRuntime = runtimesQuery.data?.find(
    (runtime) => runtime.id === "codex",
  );
  const adapterReady = codexRuntime?.availability === "available";
  const fullyReady = usable && adapterReady;
  const checking = statusQuery.isLoading || runtimesQuery.isLoading;

  async function setupRuntime() {
    setSetupError(null);
    try {
      const next = await setupMutation.mutateAsync();
      if (next.state === "ready") {
        toast.success("Codex integration is ready");
      } else {
        toast.error("Codex shared runtime did not start", {
          description: next.detail ?? undefined,
        });
      }
    } catch (cause) {
      setSetupError(cause instanceof Error ? cause.message : "Setup failed.");
      toast.error("Could not set up Codex", {
        description: cause instanceof Error ? cause.message : undefined,
      });
    }
  }

  async function launchDesktop() {
    try {
      await launchMutation.mutateAsync();
      toast.success("Opening Codex Desktop on the shared runtime");
    } catch (cause) {
      toast.error("Could not open Codex Desktop", {
        description: cause instanceof Error ? cause.message : undefined,
      });
    }
  }

  async function takeOverDesktop() {
    try {
      const next = await takeoverMutation.mutateAsync();
      setConfirmTakeover(false);
      if (isCodexSharedRuntimeUsable(next)) {
        toast.success("Codex Desktop reconnected to the shared runtime");
      } else {
        toast.error("Codex Desktop did not reconnect cleanly", {
          description: next.desktopDetectionError ?? next.detail ?? undefined,
        });
      }
    } catch (cause) {
      toast.error("Could not reconnect Codex Desktop", {
        description: cause instanceof Error ? cause.message : undefined,
      });
    }
  }

  return (
    <>
      <div
        className="space-y-4 rounded-md border border-border/70 bg-muted/20 p-4"
        data-testid="codex-shared-runtime-panel"
      >
        <div className="flex items-start gap-3">
          {fullyReady ? (
            <CircleCheck className="mt-0.5 size-5 shrink-0 text-emerald-500" />
          ) : (
            <CircleAlert className="mt-0.5 size-5 shrink-0 text-amber-500" />
          )}
          <div className="min-w-0 flex-1 space-y-1">
            <p className="text-sm font-medium">
              {checking
                ? "Checking Codex integration..."
                : conflict
                  ? "Codex Desktop runtime conflict"
                  : status?.desktopDetectionError
                    ? "Could not verify Codex Desktop"
                    : !adapterReady
                      ? "Codex ACP setup required"
                      : ready
                        ? "Codex shared runtime connected"
                        : status?.state === "unavailable"
                          ? "Codex shared runtime unavailable"
                          : "Set up Codex shared runtime"}
            </p>
            <p className="text-sm leading-5 text-muted-foreground">
              {conflict
                ? "Codex Desktop is running outside the shared runtime. Reconnect it before opening this task from Buzz."
                : !adapterReady
                  ? "Buzz will install its private Codex ACP adapter, verify it, and then start the shared app-server."
                  : usable
                    ? "Buzz and Codex Desktop can use existing tasks through the same local app-server."
                    : "Buzz requires one local Codex app-server. Fully quit Codex Desktop before enabling it, then reopen Desktop from here."}
            </p>
            {status?.url ? (
              <p className="break-all font-mono text-xs text-muted-foreground">
                {status.url}
              </p>
            ) : null}
            {conflict ? (
              <p className="text-xs text-amber-600 dark:text-amber-400">
                Found {status?.privateAppServerProcessIds.length ?? 0} private
                app-server process
                {(status?.privateAppServerProcessIds.length ?? 0) === 1
                  ? ""
                  : "es"}
                .
              </p>
            ) : null}
            {status?.desktopDetectionError ? (
              <p className="text-xs text-destructive">
                {status.desktopDetectionError}
              </p>
            ) : null}
            {status?.detail ? (
              <p className="text-xs text-destructive">{status.detail}</p>
            ) : null}
            {statusQuery.error instanceof Error ? (
              <p className="text-xs text-destructive">
                {statusQuery.error.message}
              </p>
            ) : null}
            {runtimesQuery.error instanceof Error ? (
              <p className="text-xs text-destructive">
                {runtimesQuery.error.message}
              </p>
            ) : null}
            {setupError ? (
              <p className="whitespace-pre-wrap text-xs text-destructive">
                {setupError}
              </p>
            ) : null}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {conflict && ready ? (
            <Button
              disabled={takeoverMutation.isPending}
              onClick={() => setConfirmTakeover(true)}
              size="sm"
              type="button"
              variant="destructive"
            >
              Take over Codex Desktop
            </Button>
          ) : !fullyReady ? (
            <Button
              disabled={setupMutation.isPending || checking}
              onClick={() => void setupRuntime()}
              size="sm"
              type="button"
            >
              {setupMutation.isPending
                ? adapterReady
                  ? "Starting..."
                  : "Installing Codex ACP..."
                : adapterReady
                  ? status?.enabled
                    ? "Start shared runtime"
                    : "Enable shared runtime"
                  : "Set up Codex"}
            </Button>
          ) : usable ? (
            <Button
              disabled={launchMutation.isPending}
              onClick={() => void launchDesktop()}
              size="sm"
              type="button"
              variant="outline"
            >
              <MonitorUp />
              {launchMutation.isPending ? "Opening..." : "Open Codex Desktop"}
            </Button>
          ) : null}
          <Button
            aria-label="Check Codex shared runtime again"
            disabled={
              statusQuery.isFetching ||
              runtimesQuery.isFetching ||
              takeoverMutation.isPending ||
              setupMutation.isPending
            }
            onClick={() => {
              void statusQuery.refetch();
              void runtimesQuery.refetch();
            }}
            size="icon"
            title="Check again"
            type="button"
            variant="ghost"
          >
            <RefreshCw
              className={
                statusQuery.isFetching || runtimesQuery.isFetching
                  ? "animate-spin"
                  : ""
              }
            />
          </Button>
        </div>
      </div>

      <AlertDialog
        onOpenChange={(next) => {
          if (!takeoverMutation.isPending) setConfirmTakeover(next);
        }}
        open={confirmTakeover}
      >
        <AlertDialogContent data-testid="codex-desktop-takeover-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>Reconnect Codex Desktop?</AlertDialogTitle>
            <AlertDialogDescription>
              Codex Desktop has not fully exited. Closing it may stop active
              turns and discard unsaved composer drafts. Buzz will keep the
              shared runtime at {status?.url ?? "ws://127.0.0.1:51919"} running,
              then reopen Desktop on that runtime.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button
                disabled={takeoverMutation.isPending}
                type="button"
                variant="outline"
              >
                Cancel
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                disabled={takeoverMutation.isPending}
                onClick={(event) => {
                  event.preventDefault();
                  void takeOverDesktop();
                }}
                type="button"
                variant="destructive"
              >
                {takeoverMutation.isPending
                  ? "Reconnecting..."
                  : "Close and reconnect"}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
