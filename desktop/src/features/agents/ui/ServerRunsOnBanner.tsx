import { Server } from "lucide-react";

/**
 * Read-only "this agent lives on a server" row for the Edit dialogs.
 *
 * Replaces the harness picker for server-hosted agents: where the process runs
 * is decided by the spawner, not by this device, so the only honest thing the
 * dialog can do is state it.
 */
export function ServerRunsOnBanner({
  spawnerName,
  runtime,
  pending,
}: {
  spawnerName: string;
  runtime?: string | null;
  pending: boolean;
}) {
  return (
    <div
      className="flex items-center gap-2 rounded-md border border-input bg-muted/40 px-3 py-2 text-sm"
      data-testid="server-runs-on-banner"
    >
      <Server aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span>
        Runs on <span className="font-medium">{spawnerName}</span> · Server
        {runtime ? (
          <span className="text-muted-foreground"> · {runtime}</span>
        ) : null}
      </span>
      {pending ? <ServerUpdatePendingChip className="ml-auto" /> : null}
    </div>
  );
}

/**
 * Amber chip for a prompt update that has been queued but not yet confirmed by
 * the spawner — the spawner echoes `prompt_hash` on its next status, so until
 * then the edit is in flight, not applied.
 */
export function ServerUpdatePendingChip({ className }: { className?: string }) {
  return (
    <span
      className={`rounded-full bg-amber-500/15 px-2 py-0.5 text-2xs font-medium text-amber-600${
        className ? ` ${className}` : ""
      }`}
      data-testid="server-update-pending-chip"
    >
      Update pending — server offline
    </span>
  );
}
