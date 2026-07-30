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
  pendingUpdate,
}: {
  spawnerName: string;
  runtime?: string | null;
  /** A queued prompt edit awaiting the spawner's confirmation, or null. */
  pendingUpdate: { delivered: boolean } | null;
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
      {pendingUpdate ? (
        <ServerUpdatePendingChip
          className="ml-auto"
          delivered={pendingUpdate.delivered}
        />
      ) : null}
    </div>
  );
}

/**
 * Amber chip for a prompt update that has been queued but not yet confirmed by
 * the spawner — the spawner echoes `prompt_hash` on its next status, so until
 * then the edit is in flight, not applied.
 *
 * Awaiting that echo is the *normal* path and says nothing about the server's
 * health, so the plain wording is used unless the update was never delivered
 * (`delivered === false`), which is the only case that really implies the
 * spawner could not be reached.
 */
export function ServerUpdatePendingChip({
  className,
  delivered = true,
}: {
  className?: string;
  delivered?: boolean;
}) {
  return (
    <span
      className={`rounded-full bg-amber-500/15 px-2 py-0.5 text-2xs font-medium text-amber-600${
        className ? ` ${className}` : ""
      }`}
      data-testid="server-update-pending-chip"
    >
      {delivered ? "Update pending" : "Update pending — server offline"}
    </span>
  );
}
