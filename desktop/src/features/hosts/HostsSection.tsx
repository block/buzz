import { useState } from "react";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { HostMoveSection } from "./HostMoveSection";
import { HostStartButton } from "./HostStartButton";
import { Laptop, LockKeyhole, RefreshCw } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { activeRuns } from "@/features/presence/runPresence";
import { usePresenceRuns } from "@/features/presence/usePresenceRuns";
import { HOST_REFRESH, useHostSnapshot } from "./useHostRegistration";

export function HostsSection() {
  const snapshot = useHostSnapshot();
  const { data: agents } = useManagedAgentsQuery();
  const [selectedAgent, setSelectedAgent] = useState("");
  const presence = usePresenceRuns(snapshot.rows.map((row) => row.host));
  return (
    <section aria-labelledby="hosts-heading" className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 id="hosts-heading" className="text-lg font-semibold">
            Your hosts
          </h2>
          <p className="flex items-center gap-1.5 text-sm text-muted-foreground">
            <LockKeyhole className="size-3.5" />
            Only visible to you in this community.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={snapshot.checking}
          onClick={() => window.dispatchEvent(new Event(HOST_REFRESH))}
        >
          <RefreshCw className="size-4" />
          {snapshot.checking ? "Checking…" : "Refresh"}
        </Button>
      </div>
      {snapshot.error ? (
        <p
          role="alert"
          className="rounded-md border border-destructive/30 p-3 text-sm text-destructive"
        >
          Host sync failed: {snapshot.error}. Registration requires a relay with
          host support.
        </p>
      ) : null}
      {!snapshot.rows.length ? (
        <p className="text-sm text-muted-foreground">
          {snapshot.checking
            ? "Checking this computer and its registration…"
            : "No relay-confirmed hosts to show."}
        </p>
      ) : null}
      <label className="flex flex-col gap-1 text-sm">
        Start an existing agent on a selected host
        <select
          className="rounded-md border bg-background p-2"
          value={selectedAgent}
          onChange={(event) => setSelectedAgent(event.target.value)}
        >
          <option value="">Choose an agent</option>
          {(agents ?? []).map((agent) => (
            <option key={agent.pubkey} value={agent.pubkey}>
              {agent.name}
            </option>
          ))}
        </select>
        <span className="text-xs text-muted-foreground">
          Fresh session, same identity. No workspace, configuration, keys, or
          files are transferred. Other hosts keep running.
        </span>
      </label>
      <HostMoveSection agent={selectedAgent} rows={snapshot.rows} />
      <div className="grid gap-3 md:grid-cols-2">
        {snapshot.rows.map((row) => (
          <article key={row.host} className="rounded-lg border bg-card p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="flex items-center gap-2">
                <Laptop className="size-4" />
                <h3 className="text-base font-medium">
                  {row.report?.name ?? "Registered host"}
                </h3>
              </div>
              <span className="text-xs text-muted-foreground">
                {presence.isError || !presence.data
                  ? "Presence unknown"
                  : activeRuns(presence.data[row.host], presence.now).length
                    ? "Online"
                    : "Offline"}
              </span>
            </div>
            <p className="mt-1 text-xs text-muted-foreground">
              {row.host === snapshot.local?.host
                ? `This computer · Public name: Desktop ${row.host.slice(0, 8)} · `
                : ""}
              {row.report
                ? `${row.report.os} · ${row.report.arch} · Desktop ${row.report.launcher_version}`
                : "Waiting for a capability profile"}
            </p>
            {row.report ? (
              <ul className="mt-3 flex flex-col gap-1 text-sm">
                {row.report.runtimes.map((runtime) => (
                  <li key={runtime.id} className="flex justify-between gap-2">
                    <span>{runtime.label}</span>
                    <span className="text-muted-foreground">
                      {runtime.availability.replaceAll("_", " ")} ·{" "}
                      {runtime.auth_status.replaceAll("_", " ")}
                    </span>
                  </li>
                ))}
              </ul>
            ) : null}
            <p className="mt-3 text-xs text-muted-foreground">
              {row.event
                ? `Profile updated ${new Date(row.event.created_at * 1000).toLocaleTimeString()}. `
                : ""}
              {row.report?.accepts_start
                ? "Start receiver advertised; destination rechecks configuration at launch."
                : "Remote Start is unavailable on this host."}
            </p>
            <HostStartButton
              row={row}
              agent={selectedAgent}
              online={
                presence.isError || !presence.data
                  ? undefined
                  : activeRuns(presence.data[row.host], presence.now).length > 0
              }
            />
          </article>
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        Capabilities update only when they change and remain visible offline.
        Online means Desktop is renewing its three-minute presence lease, not
        that an agent can start. History is checked across all pages before
        publishing registration or capability changes.
      </p>
    </section>
  );
}
