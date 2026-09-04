import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/shared/ui/button";
import type { RelayEvent } from "@/shared/api/types";
import type { DesktopRow, DesktopScope } from "../desktopList";
import {
  lifecycleClient,
  receiveLifecycle,
  type LifecycleOutcome,
} from "../desktopLifecycle";
import { useRelayAgentsQuery } from "../hooks";

export function DesktopLifecycleReceiver({
  scope,
}: {
  scope: DesktopScope | null;
}) {
  const { owner, community } = scope ?? {};
  useEffect(() => {
    if (!owner || !community) return;
    let active = true;
    let close: (() => void) | undefined;
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
      });
    };
    void receiveLifecycle({ owner, community }, () => active, reportError)
      .then((fn) => {
        if (active) close = fn;
        else fn();
      })
      .catch(() => {
        reportError("Desktop lifecycle receiver is unavailable.");
      });
    return () => {
      active = false;
      close?.();
      if (notification !== undefined) toast.dismiss(notification);
    };
  }, [owner, community]);
  return null;
}
function message(outcome: LifecycleOutcome) {
  switch (outcome) {
    case "running":
      return "Desktop confirmed a running local process. This does not prove model readiness.";
    case "provisioning_unavailable":
      return "Destination keyless launch provisioning is unavailable. No new process was started.";
    case "stopped":
      return "Desktop reports the agent stopped.";
    case "failed":
      return "Desktop rejected or failed the operation. No successful launch was confirmed.";
    default:
      return "Operation unconfirmed. A dispatched effect may still finish; no automatic retry will run.";
  }
}
/** Start/Move choose destination; Restart has no host picker and resolves actual current state. */
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
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [request, setRequest] = useState<RelayEvent | null>(null);
  const active = useRef(true);
  const generation = useRef(0);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
      generation.current++;
    };
  }, []);
  const run = async (action: "start" | "restart" | "move" | "retry") => {
    const token = ++generation.current;
    const valid = () => active.current && generation.current === token;
    const client = lifecycleClient(scope, valid);
    setBusy(true);
    setStatus("Checking authenticated Desktop state…");
    if (action !== "retry") setRequest(null);
    try {
      if (action === "move") {
        const outcome = await client.move(
          agent,
          destination,
          desktops.map((d) => d.id),
          (stage) => {
            if (valid()) setStatus(stage);
          },
        );
        client.check();
        setStatus(message(outcome));
      } else {
        const next =
          action === "retry"
            ? request
            : action === "start"
              ? await client.start(destination, agent)
              : await client.restart(
                  agent,
                  desktops.map((d) => d.id),
                );
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
      <h3 className="text-sm font-medium">Start, restart, or move an agent</h3>
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
      <Button
        size="sm"
        variant="outline"
        disabled={!agent || busy}
        onClick={() => void run("restart")}
      >
        Restart on current Desktop
      </Button>
      <label className="block text-xs">
        Destination for Start or Move
        <select
          aria-label="Destination Desktop"
          value={destination}
          disabled={busy}
          onChange={(e) => {
            setDestination(e.target.value);
            reset();
          }}
          className="ml-2 rounded border bg-background p-1"
        >
          <option value="">Choose a Desktop</option>
          {desktops.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name}
            </option>
          ))}
        </select>
      </label>
      <p className="text-xs text-muted-foreground">
        Start may overlap with an agent still running elsewhere until that
        Desktop reconnects. Move starts the destination only after source Stop
        is confirmed. Nothing transfers files, configuration, or keys.
      </p>
      <div className="flex gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={!agent || !destination || busy}
          onClick={() => void run("start")}
        >
          Start on destination
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!agent || !destination || busy}
          onClick={() => void run("move")}
        >
          Move to destination
        </Button>
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
