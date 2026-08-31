import { useState } from "react";
import { useCommunities } from "@/features/communities/useCommunities";
import { activeRuns } from "@/features/presence/runPresence";
import { usePresenceRuns } from "@/features/presence/usePresenceRuns";
import { useIdentityQuery } from "@/shared/api/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import type { HostRow } from "./registration";
import { moveStatus, moveUnavailable } from "./moveSelection";
import { START_REFRESH, useHostStartProgress } from "./useHostStart";

/** No optimistic removal of the source, and no browser-owned Move state machine. */
export function HostMoveSection({
  agent,
  rows,
}: {
  agent: string;
  rows: HostRow[];
}) {
  const { activeCommunity } = useCommunities();
  const { data: identity } = useIdentityQuery();
  const presence = usePresenceRuns(
    agent ? [agent, ...rows.map((r) => r.host)] : [],
  );
  const progress = useHostStartProgress();
  const [sourceId, setSourceId] = useState("");
  const [destinationId, setDestinationId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const runs = activeRuns(presence.data?.[agent], presence.now);
  const source = runs.find(
    (r) => `${agent}:${r.location?.host}:${r.run}` === sourceId,
  );
  const sourceRow = rows.find((r) => r.host === source?.location?.host);
  const destination = rows.find((r) => r.host === destinationId);
  const online = (row: HostRow) =>
    presence.isError || !presence.data
      ? undefined
      : activeRuns(presence.data[row.host], presence.now).length > 0;
  const reason = destination
    ? moveUnavailable(
        source,
        destination,
        agent,
        online(destination),
        runs.some((r) => r.location?.host === destination.host),
      )
    : "Choose a destination";
  const move = async (
    sourceRow: HostRow | undefined,
    destination: HostRow | undefined,
    run: string | undefined,
  ) => {
    const config = destination?.report?.provisioned?.find(
      (c) => c.agent === agent,
    );
    if (
      !identity ||
      !activeCommunity ||
      !sourceRow ||
      !destination ||
      !run ||
      !config
    )
      return;
    setBusy(true);
    setError(undefined);
    try {
      await invokeTauri("queue_host_move", {
        expectedOwner: identity.pubkey,
        expectedRelay: activeCommunity.relayUrl,
        sourceRegistration: sourceRow.registration,
        run,
        destinationRegistration: destination.registration,
        agent,
        runtime: config.runtime,
        revision: config.revision,
      });
      window.dispatchEvent(new Event(START_REFRESH));
    } catch (e) {
      setError(
        typeof e === "string"
          ? e
          : "Could not save Move. Refresh the hosts and retry; an unsaved Move cannot dispatch Stop.",
      );
    } finally {
      setBusy(false);
    }
  };
  if (!agent) return null;
  return (
    <div className="flex flex-col gap-2 rounded-lg border p-4">
      <h3 className="text-base font-medium">Move one active instance</h3>
      <p className="text-sm text-muted-foreground">
        Stop the selected instance first, then start this same agent in a fresh
        session on another host. No workspace, files, keys, or configuration are
        copied. Other instances keep running.
      </p>
      <label className="flex flex-col gap-1 text-sm">
        Instance to stop
        <select
          className="rounded-md border bg-background p-2"
          value={sourceId}
          onChange={(e) => setSourceId(e.target.value)}
          disabled={busy}
        >
          <option value="">Choose an active instance</option>
          {runs.map((run) => (
            <option
              key={`${run.location?.host}:${run.run}`}
              value={`${agent}:${run.location?.host}:${run.run}`}
              disabled={
                !run.location ||
                !rows.some((r) => r.host === run.location?.host)
              }
            >
              {run.location?.label ?? "Unknown host"} · Run {run.run}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1 text-sm">
        Destination
        <select
          className="rounded-md border bg-background p-2"
          value={destinationId}
          onChange={(e) => setDestinationId(e.target.value)}
          disabled={busy}
        >
          <option value="">Choose a host</option>
          {rows.map((row) => {
            const unavailable = moveUnavailable(
              source,
              row,
              agent,
              online(row),
              runs.some((r) => r.location?.host === row.host),
            );
            return (
              <option key={row.host} value={row.host} disabled={!!unavailable}>
                {row.report?.name ?? "Registered host"}
                {unavailable ? ` — ${unavailable}` : ""}
              </option>
            );
          })}
        </select>
      </label>
      <Button
        variant="outline"
        size="sm"
        disabled={
          busy || !!reason || !sourceRow || !identity || !activeCommunity
        }
        onClick={() => void move(sourceRow, destination, source?.run)}
      >
        {busy ? "Saving Move…" : "Stop selected instance and move"}
      </Button>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
      {progress.moves
        .filter((m) => m.agent === agent)
        .map((m) => {
          const live = runs.find(
            (r) =>
              r.run === m.destination_run &&
              r.location?.host === m.destination_host,
          );
          const label = (host: string) =>
            rows.find((r) => r.host === host)?.report?.name ??
            "Registered host";
          return (
            <div
              key={m.operation}
              className="flex flex-col gap-1 text-xs"
              role="status"
            >
              <p>
                {label(m.source_host)} → {label(m.destination_host)} · Run{" "}
                {m.source_run}
              </p>
              <p>{moveStatus(m.status)}</p>
              {live ? (
                <p>Live location: {live.location?.label} (matching new run)</p>
              ) : (
                <p>No matching live destination location observed.</p>
              )}
              {m.error ? <p className="text-destructive">{m.error}</p> : null}
              {m.status === "stopped_waiting_destination" ? (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={
                    busy ||
                    !rows
                      .find((r) => r.host === m.destination_host)
                      ?.report?.provisioned?.some((c) => c.agent === agent)
                  }
                  onClick={() =>
                    void move(
                      rows.find((r) => r.host === m.source_host),
                      rows.find((r) => r.host === m.destination_host),
                      m.source_run,
                    )
                  }
                >
                  Retry Move with current destination setup
                </Button>
              ) : null}
              <Button
                variant="outline"
                size="sm"
                onClick={() => window.dispatchEvent(new Event(START_REFRESH))}
              >
                Retry saved operations
              </Button>
            </div>
          );
        })}
    </div>
  );
}
