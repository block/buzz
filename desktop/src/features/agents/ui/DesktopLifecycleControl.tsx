import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/shared/ui/button";
import type { RelayEvent } from "@/shared/api/types";
import type { DesktopRow, DesktopScope } from "../desktopList";
import {
  lifecycleClient,
  type LifecycleOutcome,
  type ConfigurationChoice,
} from "../desktopLifecycle";
import { ownLifecycleReceiver } from "../desktopLifecycleReceiver";
import { useRelayAgentsQuery } from "../hooks";

export function DesktopLifecycleReceiver({
  scope,
}: {
  scope: DesktopScope | null;
}) {
  const { owner, community } = scope ?? {};
  const [attempt, retry] = useState(0);
  useEffect(() => {
    if (!owner || !community) return;
    let active = true;
    let stop = () => {};
    let notification: string | number | undefined;
    const reportError = (message: string) => {
      if (!active) return;
      // Startup mounts before the app shell: failure UI must not participate
      // in layout or displace the fixed macOS window controls. Keep one visible
      // notification for this receiver, and retire it with its owner/scope.
      notification = toast.error(message, {
        id: notification,
        duration: Infinity,
        closeButton: true,
        action: {
          label: "Retry receiver",
          onClick: () => {
            if (!active) return;
            active = false;
            stop();
            retry(attempt + 1);
          },
        },
      });
    };
    stop = ownLifecycleReceiver({ owner, community }, reportError, () => {
      if (active && notification !== undefined) toast.dismiss(notification);
    });
    return () => {
      active = false;
      stop();
      if (notification !== undefined) toast.dismiss(notification);
    };
  }, [owner, community, attempt]);
  return null;
}
function message(outcome: LifecycleOutcome) {
  switch (outcome) {
    case "running":
      return "Desktop confirmed a running local process. This does not prove model readiness.";
    case "provisioning_unavailable":
      return "Destination cannot launch this agent. No new process was started.";
    case "stopped":
      return "Desktop reports the agent stopped.";
    case "ineligible":
      return "The exact configuration is missing, changed, or unavailable. No successful launch was confirmed.";
    case "different_configuration":
      return "A different or unknown configuration is running. Use Switch runtime configuration.";
    case "failed":
      return "Desktop rejected or failed the operation. No successful launch was confirmed.";
    default:
      return "Operation unconfirmed. A dispatched effect may still finish; no automatic retry will run.";
  }
}
/** Launch choices come from owner-private destination readiness, never host inventory. */
export function DesktopLifecycleControl({
  scope,
  desktops,
}: {
  scope: DesktopScope;
  desktops: DesktopRow[];
}) {
  const agents = useRelayAgentsQuery();
  const [agent, setAgent] = useState("");
  const [destination, setDestination] = useState("");
  const [catalog, setCatalog] = useState<ConfigurationChoice[]>([]);
  const [actual, setActual] = useState("Running configuration is unknown.");
  const [refresh, setRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [request, setRequest] = useState<RelayEvent | null>(null);
  const active = useRef(true);
  const generation = useRef(0);
  useEffect(() => {
    active.current = Boolean(scope.owner && scope.community);
    setAgent("");
    setDestination("");
    setRequest(null);
    setStatus("");
    setBusy(false);
    return () => {
      active.current = false;
      generation.current++;
    };
  }, [scope.owner, scope.community]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: refresh explicitly requests a new read without changing its scope.
  useEffect(() => {
    setCatalog([]);
    setDestination("");
    setActual("Running configuration is unknown.");
    if (!agent) return;
    const token = ++generation.current;
    const valid = () => active.current && generation.current === token;
    const client = lifecycleClient(
      { owner: scope.owner, community: scope.community },
      valid,
    );
    setBusy(true);
    setStatus("Checking destination configurations…");
    void client
      .inspect(
        agent,
        desktops.map((d) => d.id),
      )
      .then((states) => {
        client.check();
        setActual(
          states
            .map(
              (state) =>
                `${desktops.find((d) => d.id === state.desktop)?.name ?? "Desktop"}: ${state.outcome === "running" ? (state.configuration ? `running configuration ${state.configuration.id}, revision ${state.configuration.revision}` : "running configuration unknown") : state.outcome === "stopped" ? "stopped" : "running state unknown"}`,
            )
            .join(" · "),
        );
      })
      .catch(() => {
        if (valid()) setActual("Running configuration is unknown.");
      });
    void client
      .catalog(
        agent,
        desktops.map((d) => d.id),
      )
      .then((entries) => {
        client.check();
        setCatalog(entries);
        setStatus(
          entries.length
            ? "Choose a configuration for the next Start. This does not change an existing process."
            : "No eligible configurations reported. Existing Stop controls remain available.",
        );
      })
      .catch(() => {
        if (valid())
          setStatus(
            "Configuration discovery is unavailable; no launch choices are offered.",
          );
      })
      .finally(() => {
        if (valid()) setBusy(false);
      });
    return () => {
      generation.current++;
    };
  }, [agent, scope.owner, scope.community, desktops, refresh]);
  useEffect(() => {
    if (!catalog.length) return;
    const expires = Math.min(...catalog.map((entry) => entry.validUntil));
    const timer = setTimeout(
      () => {
        setCatalog((entries) =>
          entries.filter((entry) => entry.validUntil > Date.now() / 1000),
        );
      },
      Math.max(0, expires * 1000 - Date.now()),
    );
    return () => clearTimeout(timer);
  }, [catalog]);
  const selected = catalog.find(
    (entry) =>
      `${entry.host}:${entry.configuration.id}:${entry.configuration.revision}` ===
      destination,
  );
  const run = async (action: "start" | "move" | "retry") => {
    const token = ++generation.current;
    const valid = () => active.current && generation.current === token;
    const client = lifecycleClient(scope, valid);
    setBusy(true);
    setStatus("Checking authenticated Desktop state…");
    setActual("Running configuration is unknown; refresh after the operation.");
    if (action !== "retry") setRequest(null);
    try {
      if (action === "move") {
        const outcome = await client.move(
          agent,
          selected?.host ?? "",
          desktops.map((d) => d.id),
          (stage) => {
            if (valid()) setStatus(stage);
          },
          selected?.configuration ?? { id: "", revision: "" },
        );
        client.check();
        setStatus(message(outcome));
      } else {
        const next =
          action === "retry"
            ? request
            : selected
              ? await client.start(selected.host, agent, selected.configuration)
              : null;
        if (!next) throw new Error("No request to retry");
        client.check();
        setRequest(next);
        setStatus("Request sent. Waiting for the Desktop’s actual result…");
        const outcome = await client.send(next);
        client.check();
        setStatus(message(outcome));
      }
    } catch (error) {
      if (valid())
        setStatus(
          error instanceof Error ? error.message : "Operation unconfirmed",
        );
    } finally {
      if (valid()) setBusy(false);
    }
  };
  const reset = () => {
    setRequest(null);
    setStatus("");
  };
  return (
    <section
      aria-label="Agent placement controls"
      className="space-y-2 rounded border p-3"
    >
      <h3 className="text-sm font-medium">Switch runtime configuration</h3>
      <label className="block text-xs">
        Agent
        <select
          aria-label="Agent to place"
          value={agent}
          disabled={busy}
          onChange={(e) => {
            setAgent(e.target.value);
            reset();
          }}
          className="ml-2 rounded border bg-background p-1"
        >
          <option value="">Choose your agent</option>
          {(agents.data ?? [])
            .filter((a) => a.ownerPubkey === scope.owner)
            .map((a) => (
              <option key={a.pubkey} value={a.pubkey}>
                {a.name}
              </option>
            ))}
        </select>
      </label>
      <p
        role="status"
        aria-label="Actual running configuration"
        className="text-xs"
      >
        {actual}
      </p>
      <Button
        size="sm"
        variant="outline"
        disabled={!agent || busy}
        onClick={() => setRefresh((n) => n + 1)}
      >
        Refresh configurations
      </Button>
      <label className="block text-xs">
        Next Start configuration
        <select
          aria-label="Runtime configuration"
          value={destination}
          disabled={busy}
          onChange={(e) => {
            setDestination(e.target.value);
            reset();
          }}
          className="ml-2 rounded border bg-background p-1"
        >
          <option value="">Choose an eligible configuration</option>
          {catalog.map((entry) => {
            const key = `${entry.host}:${entry.configuration.id}:${entry.configuration.revision}`;
            return (
              <option key={key} value={key}>
                {entry.name} ·{" "}
                {desktops.find((d) => d.id === entry.host)?.name ?? "Desktop"} ·{" "}
                {entry.runtime} · {entry.model}
                {entry.provider ? ` (${entry.provider})` : ""}
              </option>
            );
          })}
        </select>
      </label>
      <p className="text-xs text-muted-foreground">
        Start may overlap with an agent still running elsewhere until that
        Desktop reconnects. Switch checks the target, confirms Stop, then starts
        fresh—even on the same Desktop. No files, settings, sessions, or keys
        transfer.
      </p>
      <div className="flex gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={!agent || !selected || busy}
          onClick={() => void run("start")}
        >
          Start on destination
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!agent || !selected || busy}
          onClick={() => void run("move")}
        >
          Switch runtime configuration
        </Button>
        {busy && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              generation.current++;
              setBusy(false);
              setRequest(null);
              setStatus(
                "Stopped waiting. Dispatched operations may still finish; no later launch will be requested by this control.",
              );
            }}
          >
            Cancel waiting
          </Button>
        )}
        {request && (
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => void run("retry")}
          >
            Retry same request
          </Button>
        )}
      </div>
      {status && (
        <p role="status" className="text-xs">
          {status}
        </p>
      )}
    </section>
  );
}
