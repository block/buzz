import { CircleAlert, CircleCheck, MonitorUp, RefreshCw } from "lucide-react";
import { toast } from "sonner";

import {
  useCodexSharedRuntimeQuery,
  useEnableCodexSharedRuntimeMutation,
  useLaunchCodexDesktopSharedMutation,
} from "@/features/agents/codexSharedRuntimeHooks";
import { Button } from "@/shared/ui/button";

export function CodexSharedRuntimePanel({
  enabled = true,
}: {
  enabled?: boolean;
}) {
  const statusQuery = useCodexSharedRuntimeQuery({ enabled });
  const enableMutation = useEnableCodexSharedRuntimeMutation();
  const launchMutation = useLaunchCodexDesktopSharedMutation();
  const status = statusQuery.data;
  const ready = status?.state === "ready";

  async function enableRuntime() {
    try {
      const next = await enableMutation.mutateAsync();
      if (next.state === "ready") {
        toast.success("Codex shared runtime is ready");
      } else {
        toast.error("Codex shared runtime did not start", {
          description: next.detail ?? undefined,
        });
      }
    } catch (cause) {
      toast.error("Could not enable Codex shared runtime", {
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

  return (
    <div
      className="space-y-4 rounded-md border border-border/70 bg-muted/20 p-4"
      data-testid="codex-shared-runtime-panel"
    >
      <div className="flex items-start gap-3">
        {ready ? (
          <CircleCheck className="mt-0.5 size-5 shrink-0 text-emerald-500" />
        ) : (
          <CircleAlert className="mt-0.5 size-5 shrink-0 text-amber-500" />
        )}
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-sm font-medium">
            {statusQuery.isLoading
              ? "Checking Codex shared runtime..."
              : ready
                ? "Codex shared runtime connected"
                : status?.state === "unavailable"
                  ? "Codex shared runtime unavailable"
                  : "Set up Codex shared runtime"}
          </p>
          <p className="text-sm leading-5 text-muted-foreground">
            {ready
              ? "Buzz and Codex Desktop can use existing tasks through the same local app-server."
              : "Buzz requires one local Codex app-server. Fully quit Codex Desktop before enabling it, then reopen Desktop from here."}
          </p>
          {status?.url ? (
            <p className="break-all font-mono text-xs text-muted-foreground">
              {status.url}
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
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {!ready ? (
          <Button
            disabled={enableMutation.isPending}
            onClick={() => void enableRuntime()}
            size="sm"
            type="button"
          >
            {enableMutation.isPending
              ? "Starting..."
              : status?.enabled
                ? "Start shared runtime"
                : "Enable shared runtime"}
          </Button>
        ) : (
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
        )}
        <Button
          aria-label="Check Codex shared runtime again"
          disabled={statusQuery.isFetching}
          onClick={() => void statusQuery.refetch()}
          size="icon"
          title="Check again"
          type="button"
          variant="ghost"
        >
          <RefreshCw className={statusQuery.isFetching ? "animate-spin" : ""} />
        </Button>
      </div>
    </div>
  );
}
