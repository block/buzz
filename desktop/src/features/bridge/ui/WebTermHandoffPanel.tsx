// Web body of the "Pass thread context to Buzz Term" popover: agents + harnesses come from the member's own desktop
// (relay bridge), and Go sends a `term.handoff` command to it. On success the web opens Buzz term and attaches.
import { useNavigate } from "@tanstack/react-router";
import * as React from "react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import { CommandError } from "../commander";
import { useBridgeAgents } from "../useBridgeAgents";
import {
  NO_DESKTOP_AGENTS_LABEL,
  handoffAgentsFor,
  handoffHarnesses,
  parseHandoffResult,
  planAttach,
  setPendingTermHandoff,
} from "../webTermHandoff";
import { useRemoteHostsFor } from "./WebRemoteSettings";

type Props = { channelId: string; threadId: string; onDone: () => void };

export function WebTermHandoffPanel({ channelId, threadId, onDone }: Props) {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const { hosts, reachOf } = useRemoteHostsFor(b);
  const navigate = useNavigate();
  const [agent, setAgent] = React.useState("");
  const [harness, setHarness] = React.useState<"claude" | "codex">("claude");
  const [cmdId, setCmdId] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [note, setNote] = React.useState<string | null>(null);
  const handled = React.useRef<string | null>(null);

  const agents = React.useMemo(() => handoffAgentsFor(b.snapshot, channelId), [b.snapshot, channelId]);
  const harnesses = handoffHarnesses(b.heartbeat);
  React.useEffect(() => {
    if (agent && !agents.some((a) => a.pubkey === agent)) setAgent("");
  }, [agents, agent]);
  React.useEffect(() => {
    const first = harnesses?.find((h) => h.available)?.id;
    if (first) setHarness((cur) => (harnesses?.find((h) => h.id === cur)?.available ? cur : first));
  }, [harnesses]);

  const status = cmdId && b.commander ? b.commander.statusOf(cmdId) : null;
  const busy = status === "pending" || status === "accepted";
  const desktopOnline = b.presence === "online";
  const supported = b.commander?.supports("term.handoff") ?? false;
  const selectedHarness = harnesses?.find((h) => h.id === harness);
  const canGo = !!agent && !!selectedHarness?.available && !busy && desktopOnline && supported && !!b.commander;

  // Ack arrives: navigate to Buzz term and attach, or explain what to enable.
  React.useEffect(() => {
    if (!cmdId || handled.current === cmdId || !status) return;
    if (status === "error" || status === "timeout") {
      handled.current = cmdId;
      const r = b.results.get(cmdId);
      setError(status === "timeout" ? "Your desktop didn't answer in time. Check the thread on your desktop." : (r?.message ?? "The desktop couldn't start the session."));
      return;
    }
    if (status !== "ok") return;
    handled.current = cmdId;
    const handoff = parseHandoffResult(b.results.get(cmdId)?.result);
    const plan = planAttach(hosts[0], !!b.heartbeat?.remote, hosts[0] ? reachOf(hosts[0]) : "unknown");
    if (handoff && plan.attach) {
      setPendingTermHandoff(handoff);
      onDone();
      void navigate({ to: "/buzz-term" });
    } else {
      setNote(plan.attach ? "The Buzz Term session started on your desktop." : plan.message);
    }
  });

  async function go() {
    if (!canGo || !b.commander) return;
    setError(null);
    setNote(null);
    try {
      const id = await b.commander.send(
        "term.handoff",
        { agentPubkey: agent, channelId, threadId, harness },
        { confirmed: true, label: "Buzz Term handoff" },
      );
      setCmdId(id);
    } catch (e) {
      setError(e instanceof CommandError ? e.message : String(e instanceof Error ? e.message : e));
    }
  }

  const selectCls = "rounded-md border border-border bg-background px-2 py-1.5";
  return (
    <>
      <div>
        <div className="text-sm font-semibold">Pass thread context to Buzz Term</div>
        <p className="mt-0.5 text-2xs text-muted-foreground">
          Your desktop's agent summarizes this thread into a Term handoff, then a Buzz Term session starts with it.
        </p>
      </div>
      {!desktopOnline ? (
        <p className="text-sm text-muted-foreground" data-testid="thread-term-session-offline">
          {b.presence === "unknown" ? "Looking for your desktop…" : "Your desktop is offline. Open Buzz on it to continue."}
        </p>
      ) : !supported && b.commander ? (
        <p className="text-sm text-muted-foreground" data-testid="thread-term-session-unsupported">
          Update the desktop app to start Buzz Term sessions from here.
        </p>
      ) : null}
      <label className="flex flex-col gap-1 text-sm">
        <span className="text-muted-foreground">Agent</span>
        {agents.length === 0 ? (
          <p className="rounded-md border border-dashed border-border px-2 py-1.5 text-sm text-muted-foreground" data-testid="thread-term-session-agent-empty">
            {NO_DESKTOP_AGENTS_LABEL}
          </p>
        ) : (
          <select className={selectCls} data-testid="thread-term-session-agent" value={agent} onChange={(e) => setAgent(e.target.value)}>
            <option value="">Select agent…</option>
            {agents.map((a) => (
              <option key={a.pubkey} value={a.pubkey}>
                {a.name || a.pubkey.slice(0, 12)}
              </option>
            ))}
          </select>
        )}
      </label>
      <label className="flex flex-col gap-1 text-sm">
        <span className="text-muted-foreground">Harness</span>
        {harnesses ? (
          <select className={selectCls} data-testid="thread-term-session-harness" value={harness} onChange={(e) => setHarness(e.target.value as "claude" | "codex")}>
            {harnesses.map((h) => (
              <option key={h.id} value={h.id} disabled={!h.available}>
                {h.label}
                {h.available ? "" : " (not installed)"}
              </option>
            ))}
          </select>
        ) : (
          <p className="rounded-md border border-dashed border-border px-2 py-1.5 text-sm text-muted-foreground" data-testid="thread-term-session-harness-unknown">
            Checking what's installed on your desktop…
          </p>
        )}
        {selectedHarness && !selectedHarness.available ? (
          <span className="text-2xs text-muted-foreground" data-testid="thread-term-session-harness-hint">
            {selectedHarness.hint}
          </span>
        ) : null}
      </label>
      {busy ? (
        <p className="text-2xs text-muted-foreground" data-testid="thread-term-session-progress">
          {status === "accepted" ? "Your desktop's agent is writing the handoff…" : "Sending to your desktop…"}
        </p>
      ) : null}
      {error ? (
        <p className="text-sm text-destructive" data-testid="thread-term-session-error">
          {error}
        </p>
      ) : null}
      {note ? (
        <p className="text-sm" data-testid="thread-term-session-note">
          {note}
        </p>
      ) : null}
      <div className="flex justify-end">
        <Button data-testid="thread-term-session-go" disabled={!canGo} onClick={() => void go()} size="sm" type="button">
          {busy ? "Working…" : "Go"}
        </Button>
      </div>
    </>
  );
}
