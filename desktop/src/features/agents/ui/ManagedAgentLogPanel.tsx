import { CircleAlert } from "lucide-react";

import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Skeleton } from "@/shared/ui/skeleton";
import { CopyButton } from "./CopyButton";
import { describeLogFile } from "./agentUi";

export function ManagedAgentLogPanel({
  error,
  isLoading,
  logContent,
  selectedAgent,
  variant = "section",
}: {
  error: Error | null;
  isLoading: boolean;
  logContent: string | null;
  selectedAgent: ManagedAgent | null;
  variant?: "inline" | "section";
}) {
  const isInline = variant === "inline";
  const logFileLabel = selectedAgent
    ? describeLogFile(selectedAgent.logPath)
    : null;

  if (!selectedAgent && isInline) {
    return null;
  }

  return (
    <section
      className={cn(
        "flex flex-col",
        isInline
          ? "h-full min-h-0"
          : "rounded-[28px] border border-border/70 bg-card/90 p-5 shadow-xs",
      )}
    >
      {!selectedAgent ? (
        <div className="flex shrink-0 flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <h3 className="text-sm font-semibold tracking-tight">
              Harness log
            </h3>
            <p className="text-sm text-muted-foreground">
              Select a local agent to inspect recent output.
            </p>
          </div>
        </div>
      ) : null}

      {!selectedAgent ? (
        <div
          className={cn(
            "mt-4 rounded-xl border border-dashed border-border/80 bg-background/70 px-6 py-10 text-center",
            isInline && "flex min-h-0 flex-1 flex-col justify-center",
          )}
        >
          <p className="text-sm font-semibold tracking-tight">
            No local agent selected
          </p>
          <p className="mt-2 text-sm text-muted-foreground">
            Pick a managed agent to view the latest ACP log output.
          </p>
        </div>
      ) : isLoading ? (
        <div
          className={cn(
            "overflow-hidden rounded-xl border border-border/70 bg-[#17171d] text-xs text-zinc-100",
            isInline ? "flex min-h-0 flex-1 flex-col" : "mt-4",
          )}
        >
          <HarnessLogHeader
            logContent={logContent ?? ""}
            logFileLabel={logFileLabel ?? ""}
            selectedAgent={selectedAgent}
          />
          <div className="p-4">
            <Skeleton className="h-4 w-48 bg-white/10" />
            <Skeleton className="mt-3 h-4 w-full bg-white/10" />
            <Skeleton className="mt-2 h-4 w-full bg-white/10" />
            <Skeleton className="mt-2 h-4 w-3/4 bg-white/10" />
          </div>
        </div>
      ) : (
        <div
          className={cn(
            "overflow-hidden rounded-xl border border-border/70 bg-[#17171d] text-xs text-zinc-100",
            isInline && "flex min-h-0 flex-1 flex-col",
            !isInline && "mt-4",
          )}
        >
          <HarnessLogHeader
            logContent={logContent ?? ""}
            logFileLabel={logFileLabel ?? ""}
            selectedAgent={selectedAgent}
          />
          <pre
            className={cn(
              "overflow-auto whitespace-pre-wrap px-4 py-4",
              isInline ? "min-h-0 flex-1" : "max-h-88",
            )}
            data-testid="managed-agent-log-content"
          >
            {logContent?.trim() ? logContent : "No log output yet."}
          </pre>
        </div>
      )}

      {error ? (
        <p className="mt-4 inline-flex items-center gap-2 rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          <CircleAlert className="h-4 w-4" />
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function HarnessLogHeader({
  logContent,
  logFileLabel,
  selectedAgent,
}: {
  logContent: string;
  logFileLabel: string;
  selectedAgent: ManagedAgent;
}) {
  const fileTitle = `${selectedAgent.name} · ${logFileLabel}`;

  return (
    <div className="flex min-h-12 items-center justify-between gap-3 border-b border-white/10 px-3 py-2">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="min-w-0 truncate text-2xs font-semibold uppercase tracking-[0.18em] text-zinc-300">
          Harness log
        </span>
        <span
          className="min-w-0 truncate font-mono text-2xs text-zinc-500"
          title={fileTitle}
        >
          {selectedAgent.name} · {logFileLabel}
        </span>
      </div>
      <CopyButton
        className="h-6 rounded-md bg-black/40 px-2 text-zinc-300 hover:bg-black/70 hover:text-white"
        label="Copy log"
        size="xs"
        value={logContent}
        variant="ghost"
      />
    </div>
  );
}
