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

  if (!selectedAgent && isInline) {
    return null;
  }

  return (
    <section
      className={cn(
        isInline
          ? ""
          : "rounded-[28px] border border-border/70 bg-card/90 p-5 shadow-xs",
      )}
    >
      <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h3 className="text-sm font-semibold tracking-tight">Harness log</h3>
          <p className="text-sm text-muted-foreground">
            {selectedAgent
              ? `${selectedAgent.name} · ${describeLogFile(selectedAgent.logPath)}`
              : "Select a local agent to inspect recent output."}
          </p>
        </div>
        {selectedAgent ? (
          <CopyButton label="Copy log" value={logContent ?? ""} />
        ) : null}
      </div>

      {!selectedAgent ? (
        <div className="mt-4 rounded-xl border border-dashed border-border/80 bg-background/70 px-6 py-10 text-center">
          <p className="text-sm font-semibold tracking-tight">
            No local agent selected
          </p>
          <p className="mt-2 text-sm text-muted-foreground">
            Pick a managed agent to view the latest ACP log output.
          </p>
        </div>
      ) : isLoading ? (
        <div className="mt-4 rounded-xl border border-border/70 bg-background/80 p-4">
          <Skeleton className="h-4 w-48" />
          <Skeleton className="mt-3 h-4 w-full" />
          <Skeleton className="mt-2 h-4 w-full" />
          <Skeleton className="mt-2 h-4 w-3/4" />
        </div>
      ) : (
        <div className="mt-4 overflow-hidden rounded-xl border border-border/70 bg-[#17171d] text-xs text-zinc-100">
          <div className="flex items-center justify-between border-b border-white/10 px-4 py-2 text-2xs uppercase tracking-[0.18em] text-zinc-400">
            <span>{selectedAgent.name}</span>
            <span>{selectedAgent.status}</span>
          </div>
          <pre
            className={cn(
              "overflow-auto whitespace-pre-wrap px-4 py-4",
              isInline ? "max-h-[18rem]" : "max-h-[22rem]",
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
