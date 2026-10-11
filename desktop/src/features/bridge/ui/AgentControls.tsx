// Web controls for one agent: start / stop / message, each destructive one behind an explicit confirmation step.
import * as React from "react";

import { CMD_MAX_TEXT, nowS, type AgentSummary, type CommandType } from "../protocol";
import type { BridgeCommander, SentCommand } from "../commander";
import { type DirectAgents, directErrorCopy, isDirectDown } from "../directAgents";
import type { BridgeState } from "../webClient";

const VERB: Record<string, string> = {
  "agent.start": "Start",
  "agent.stop": "Stop",
  "agent.message": "Message",
};

export function AgentControls(props: {
  agent: AgentSummary;
  commander: BridgeCommander;
  bridge: BridgeState;
  /** A live signed direct connection: start/stop/message use it and fall back to the relay if it drops. */
  direct?: DirectAgents | null;
}) {
  const { agent, commander, bridge, direct } = props;
  const [, tick] = React.useReducer((x: number) => x + 1, 0);
  React.useEffect(() => commander.subscribe(tick), [commander]);
  React.useEffect(() => {
    const t = window.setInterval(tick, 2000);
    return () => window.clearInterval(t);
  }, []);
  const [confirming, setConfirming] = React.useState<null | { type: CommandType }>(null);
  const [text, setText] = React.useState("");
  const [channelId, setChannelId] = React.useState(agent.activeTurns[0]?.channelId ?? "");
  const [error, setError] = React.useState<string | null>(null);
  const online = bridge.presence === "online";
  const running = agent.status === "running";
  const mine: SentCommand[] = [...commander.sent.values()]
    .filter((c) => c.agentPubkey === agent.pubkey)
    .sort((a, b) => b.ts - a.ts)
    .slice(0, 5);

  const fire = async (type: CommandType, confirmed: boolean) => {
    setError(null);
    if (direct?.live) {
      try {
        if (type === "agent.message") await direct.message(agent.pubkey, channelId, text);
        else if (type === "agent.start") await direct.start(agent.pubkey);
        else await direct.stop(agent.pubkey);
        setConfirming(null);
        if (type === "agent.message") setText("");
        return;
      } catch (e) {
        if (!isDirectDown(e)) {
          setError(directErrorCopy(e));
          setConfirming(null);
          return;
        }
        // the direct path dropped: fall through to the relay with the same command
      }
    }
    try {
      if (type === "agent.message")
        await commander.send(type, { agentPubkey: agent.pubkey, channelId, text }, { confirmed, label: `Message: ${text.slice(0, 40)}` });
      else await commander.send(type, { agentPubkey: agent.pubkey }, { confirmed, label: VERB[type] });
      setConfirming(null);
      if (type === "agent.message") setText("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setConfirming(null);
    }
  };
  const channels = agent.activeTurns;
  const canMessage = commander.supports("agent.message");
  const btn = { fontSize: 12, padding: "4px 10px", marginRight: 6 } as const;
  const disabled = !online;
  return (
    <div data-testid="agent-controls" style={{ margin: "8px 0", display: "grid", gap: 8 }}>
      {!online && (
        <p style={{ fontSize: 12, opacity: 0.7, margin: 0 }}>Controls are off while your desktop is offline.</p>
      )}
      <div>
        {!running && commander.supports("agent.start") && (
          <button type="button" data-testid="ctl-start" disabled={disabled} style={btn} onClick={() => void fire("agent.start", false)}>
            Start
          </button>
        )}
        {running && commander.supports("agent.stop") && (
          <button type="button" data-testid="ctl-stop" disabled={disabled} style={btn} onClick={() => setConfirming({ type: "agent.stop" })}>
            Stop…
          </button>
        )}
      </div>
      {canMessage && running && (
        <div style={{ display: "grid", gap: 4 }}>
          <input
            data-testid="ctl-channel"
            placeholder="Channel id"
            value={channelId}
            onChange={(e) => setChannelId(e.target.value)}
            list="ctl-channels"
            style={{ fontSize: 12 }}
          />
          <datalist id="ctl-channels">
            {channels.map((c) => (
              <option key={c.channelId} value={c.channelId}>
                {c.channelName}
              </option>
            ))}
          </datalist>
          <textarea
            data-testid="ctl-text"
            placeholder={`Message ${agent.name}…`}
            value={text}
            maxLength={CMD_MAX_TEXT}
            onChange={(e) => setText(e.target.value)}
            rows={2}
            style={{ fontSize: 12 }}
          />
          <div>
            <button
              type="button"
              data-testid="ctl-send"
              disabled={disabled || !text.trim() || !channelId.trim()}
              style={btn}
              onClick={() => setConfirming({ type: "agent.message" })}
            >
              Send…
            </button>
          </div>
        </div>
      )}
      {confirming && (
        <div data-testid="ctl-confirm" role="alertdialog" style={{ border: "1px solid #c0392b", borderRadius: 8, padding: 8, fontSize: 12 }}>
          {confirming.type === "agent.stop"
            ? `Stop ${agent.name} on your desktop? Work in progress will be interrupted.`
            : `Send this message as you to channel ${channelId}, mentioning ${agent.name}?\n“${text.slice(0, 200)}”`}
          <div style={{ marginTop: 6 }}>
            <button type="button" data-testid="ctl-confirm-yes" style={btn} onClick={() => void fire(confirming.type, true)}>
              Confirm
            </button>
            <button type="button" data-testid="ctl-confirm-no" style={btn} onClick={() => setConfirming(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {error && (
        <p data-testid="ctl-error" style={{ fontSize: 12, color: "#c0392b", margin: 0 }}>
          {error}
        </p>
      )}
      {mine.length > 0 && (
        <ul data-testid="ctl-status" style={{ listStyle: "none", padding: 0, margin: 0, fontSize: 12 }}>
          {mine.map((c) => {
            const st = commander.statusOf(c.id, nowS());
            const r = bridge.results.get(c.id);
            return (
              <li key={c.id} data-status={st}>
                {c.label}: {st === "pending" ? "sending…" : st === "accepted" ? "desktop accepted…" : st === "ok" ? `done${r?.message ? ` (${r.message})` : ""}` : st === "timeout" ? "desktop did not respond" : `failed${r?.message ? `: ${r.message}` : ""}`}
              </li>
            );
          })}
        </ul>
      )}
      <p style={{ fontSize: 11, opacity: 0.55, margin: 0 }}>
        Permission prompts can't be answered remotely yet.
      </p>
    </div>
  );
}
