import { useState } from "react";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import type { HostRow } from "./registration";
import { startStatus, startUnavailable } from "./startSelection";
import { START_REFRESH, useHostStartProgress } from "./useHostStart";
import { usePresenceRuns } from "@/features/presence/usePresenceRuns";
import { activeRuns } from "@/features/presence/runPresence";

export function HostStartButton({
  row,
  agent,
  online,
}: {
  row: HostRow;
  agent: string;
  online?: boolean;
}) {
  const { activeCommunity } = useCommunities();
  const { data: identity } = useIdentityQuery();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const progress = useHostStartProgress();
  const operation = progress.operations
    .filter(
      (op) =>
        op.action !== "stop" &&
        op.agent === agent &&
        op.host === row.host &&
        op.current,
    )
    .sort((a, b) => a.created_at - b.created_at)
    .at(-1);
  const runs = usePresenceRuns(agent ? [agent] : []);
  const live =
    operation &&
    activeRuns(runs.data?.[agent], runs.now).find(
      (run) => run.run === operation.run && run.location?.host === row.host,
    );
  const reason = startUnavailable(row, agent, online);
  const config = row.report?.provisioned?.find((c) => c.agent === agent);
  const start = async (fresh: boolean) => {
    if (!config || !identity || !activeCommunity) return;
    setBusy(true);
    setError(undefined);
    try {
      await invokeTauri("queue_host_start", {
        expectedOwner: identity.pubkey,
        expectedRelay: activeCommunity.relayUrl,
        registration: row.registration,
        agent,
        runtime: config.runtime,
        revision: config.revision,
        newAttemptAfter: fresh ? operation?.operation : undefined,
      });
      window.dispatchEvent(new Event(START_REFRESH));
    } catch {
      setError(
        fresh
          ? "A new session requires a signed confirmed Stop of the prior run (or a rejected Start). No replacement was queued. Refresh and retry the saved operation if its outcome is unknown."
          : "Could not save Start. Check destination registration and retry; no unsaved command was sent.",
      );
    } finally {
      setBusy(false);
    }
  };
  const disabled = busy || !!reason || !identity || !activeCommunity;
  return (
    <div className="mt-3 flex flex-col gap-2">
      <Button
        size="sm"
        variant="outline"
        disabled={disabled}
        onClick={() => void start(false)}
      >
        {busy
          ? "Saving Start…"
          : operation
            ? "Retry saved Start"
            : "Start on this host"}
      </Button>
      {operation ? (
        <Button
          size="sm"
          variant="outline"
          disabled={disabled}
          onClick={() => void start(true)}
        >
          {operation.status === "rejected"
            ? "Create new Start attempt"
            : "Start new session after confirmed Stop"}
        </Button>
      ) : null}
      {reason ? (
        <p className="text-xs text-muted-foreground">
          {reason}. Use Agents on that Desktop to complete setup; keys and files
          are not copied.
        </p>
      ) : null}
      {operation ? (
        <p role="status" className="text-xs text-muted-foreground">
          {startStatus(operation.status)}
          {live
            ? ` · Live location: ${live.location?.label} (same run)`
            : " · No matching live location observed"}
        </p>
      ) : null}
      {error || operation?.error || progress.error ? (
        <p role="alert" className="text-xs text-destructive">
          {error ?? operation?.error ?? progress.error}
        </p>
      ) : null}
    </div>
  );
}
