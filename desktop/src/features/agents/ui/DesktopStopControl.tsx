import { useEffect, useRef, useState } from "react";
import { useRelayAgentsQuery } from "../hooks";
import {
  prepareStop,
  readStopOutcome,
  receiveStops,
  sendStop,
} from "../desktopStop";
import type { DesktopScope, DesktopRow } from "../desktopList";
import type { RelayEvent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

/** App-scoped live receiver; no historical requests are loaded on mount. */
export function DesktopStopReceiver({ scope }: { scope: DesktopScope | null }) {
  const [error, setError] = useState("");
  const { owner, community } = scope ?? {};
  useEffect(() => {
    if (!owner || !community) return;
    let active = true;
    let close: (() => void) | undefined;
    setError("");
    void receiveStops({ owner, community }, () => active, setError)
      .then((unsubscribe) => {
        if (active) close = unsubscribe;
        else unsubscribe();
      })
      .catch(() => {
        if (active)
          setError("Remote Stop receiver is unavailable on this Desktop.");
      });
    return () => {
      active = false;
      close?.();
    };
  }, [owner, community]);
  return error ? (
    <p role="status" className="text-xs text-muted-foreground">
      {error}
    </p>
  ) : null;
}

/** Deliberately selects a host, not an inferred running location or presence. */
export function DesktopStopControl({
  scope,
  desktop,
}: {
  scope: DesktopScope;
  desktop: DesktopRow;
}) {
  const agents = useRelayAgentsQuery();
  const owned = (agents.data ?? []).filter(
    (agent) => agent.ownerPubkey === scope.owner,
  );
  const [agent, setAgent] = useState("");
  const [request, setRequest] = useState<RelayEvent | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const run = async (retry: boolean) => {
    setBusy(true);
    let current = retry ? request : null;
    try {
      current ??= await prepareStop(
        scope,
        desktop.id,
        agent,
        () => active.current,
      );
      if (!active.current) return;
      setRequest(current);
      setMessage("Stop requested. Waiting for this Desktop’s result…");
      try {
        await sendStop(scope, current, () => active.current);
      } catch {
        if (active.current)
          setMessage(
            "Delivery unconfirmed. Checking for this Desktop’s result…",
          );
      }
      for (let attempt = 0; attempt < 15 && active.current; attempt++) {
        const outcome = await readStopOutcome(
          scope,
          current,
          () => active.current,
        );
        if (!active.current) return;
        if (outcome === "stopped") {
          setMessage(`Stop confirmed by ${desktop.name}.`);
          return;
        }
        if (outcome === "failed") {
          setMessage(
            `Stop failed on ${desktop.name}. No success was confirmed.`,
          );
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      if (active.current)
        setMessage(
          "Stop unconfirmed. This Desktop may be unavailable; its agents may still be running.",
        );
    } catch {
      if (active.current)
        setMessage("Stop unconfirmed. No successful result could be read.");
    } finally {
      if (active.current) setBusy(false);
    }
  };
  return (
    <div className="mt-2 space-y-2">
      <label className="block text-xs">
        Agent to stop on {desktop.name}
        <select
          aria-label={`Agent to stop on ${desktop.name}`}
          className="ml-2 rounded border bg-background p-1 text-sm"
          value={agent}
          disabled={busy}
          onChange={(event) => {
            setAgent(event.target.value);
            setRequest(null);
            setMessage("");
          }}
        >
          <option value="">Choose your agent</option>
          {owned.map((item) => (
            <option key={item.pubkey} value={item.pubkey}>
              {item.name}
            </option>
          ))}
        </select>
      </label>
      <p className="text-xs text-muted-foreground">
        Stops only this agent on this Desktop in this community. This list does
        not establish where it is running.
      </p>
      {agents.isError && <p role="status">Your agent list is unavailable.</p>}
      <Button
        size="sm"
        variant="outline"
        disabled={!agent || busy}
        onClick={() => void run(false)}
      >
        Stop on {desktop.name}
      </Button>
      {request && (
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          onClick={() => void run(true)}
        >
          Retry same Stop
        </Button>
      )}
      {message && (
        <p role="status" className="text-xs">
          {message}
        </p>
      )}
    </div>
  );
}
