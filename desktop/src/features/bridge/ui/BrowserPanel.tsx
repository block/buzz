// Web Browsers page body: the desktop's Browsers list (synced over the bridge), Add browser (confirmed browser.open), per-browser
// See screenshot (view-only), Remove (confirmed) and the robot (Observe/Drive) control, shown only when the host allows remote agent management.
import * as React from "react";

import { nowS } from "../protocol";
import type { BridgeCommander } from "../commander";
import type { BridgeState } from "../webClient";

export type ChannelOption = { id: string; name: string };

export function BrowserPanel(props: { commander: BridgeCommander; bridge: BridgeState; channels?: ChannelOption[] }) {
  const { commander, bridge, channels = [] } = props;
  const agentsAllowed = bridge.heartbeat?.remote?.agents === true && commander.supports("browser.agent");
  const [robot, setRobot] = React.useState<null | { id: string; agent: string; mode: "off" | "observe" | "drive"; channel: string }>(null);
  const [, tick] = React.useReducer((x: number) => x + 1, 0);
  React.useEffect(() => commander.subscribe(tick), [commander]);
  React.useEffect(() => {
    const t = window.setInterval(tick, 1500);
    return () => window.clearInterval(t);
  }, []);
  const [url, setUrl] = React.useState("");
  const [confirming, setConfirming] = React.useState<null | { type: "open" } | { type: "close"; id: string; host: string }>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [shotCmd, setShotCmd] = React.useState<Record<string, string>>({});
  const online = bridge.presence === "online";
  if (!commander.supports("browser.open") && bridge.browserSessions.length === 0) return null;
  const btn = { fontSize: 12, padding: "4px 10px", marginRight: 6 } as const;
  const wrap = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setConfirming(null);
  };
  const busy = commander.shotBusy();
  const recent = [...commander.sent.values()]
    .filter((c) => c.type.startsWith("browser."))
    .sort((a, b) => b.ts - a.ts)
    .slice(0, 3);
  return (
    <div data-testid="browser-panel" style={{ marginTop: 16, borderTop: "1px solid #8884", paddingTop: 8 }}>
      <h3 style={{ fontSize: 14, margin: "0 0 6px" }}>Browsers on your desktop</h3>
      <p style={{ fontSize: 11, opacity: 0.6, margin: "0 0 6px" }}>
        These are the browsers on your desktop's Browsers page. Add one here and it appears there; remove one on either side and it disappears on both. Screenshots are taken only when you ask.
      </p>
      <div style={{ display: "flex", gap: 6 }}>
        <input
          data-testid="br-url"
          placeholder="https://…"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          style={{ flex: 1, fontSize: 12 }}
          inputMode="url"
        />
        <button
          type="button"
          data-testid="br-open"
          disabled={!online || !url.trim() || !commander.supports("browser.open")}
          style={btn}
          onClick={() => setConfirming({ type: "open" })}
        >
          Add browser…
        </button>
      </div>
      {confirming && (
        <div data-testid="br-confirm" role="alertdialog" style={{ border: "1px solid #c0392b", borderRadius: 8, padding: 8, fontSize: 12, marginTop: 6 }}>
          {confirming.type === "open"
            ? `Add ${url.trim().slice(0, 200)} as a browser on your desktop? It will also show on your desktop's Browsers page.`
            : `Remove ${confirming.host} from your desktop's Browsers page?`}
          <div style={{ marginTop: 6 }}>
            <button
              type="button"
              data-testid="br-confirm-yes"
              style={btn}
              onClick={() =>
                void wrap(async () => {
                  if (confirming.type === "open") {
                    await commander.send("browser.open", { url: url.trim() }, { confirmed: true, label: `Add ${url.trim().slice(0, 40)}` });
                    setUrl("");
                  } else
                    await commander.send("browser.close", { sessionId: confirming.id }, { confirmed: true, label: `Close ${confirming.host}` });
                })
              }
            >
              Confirm
            </button>
            <button type="button" style={btn} onClick={() => setConfirming(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {error && (
        <p data-testid="br-error" style={{ fontSize: 12, color: "#c0392b", margin: "6px 0 0" }}>
          {error}
        </p>
      )}
      <ul style={{ listStyle: "none", padding: 0, margin: "8px 0 0" }}>
        {bridge.browserSessions.map((s) => {
          const shot = bridge.shots.get(s.sessionId);
          const cid = shotCmd[s.sessionId];
          const st = cid ? commander.statusOf(cid, nowS()) : null;
          const r = cid ? bridge.results.get(cid) : undefined;
          return (
            <li key={s.sessionId} data-testid="br-session" style={{ marginBottom: 10 }}>
              <div style={{ fontSize: 13 }}>
                {s.title || s.host}{" "}
                {s.status === "cold" && <span data-testid="br-cold" title="Not started on the desktop yet; a screenshot starts it" style={{ fontSize: 10, opacity: 0.7 }}>· Cold</span>}
                {s.status === "live" && <span style={{ fontSize: 10, opacity: 0.7 }}>· Live</span>}
                <span style={{ fontSize: 10, opacity: 0.6 }}> · {s.origin === "web" ? "added from web" : "added on desktop"}</span>
              </div>
              <div style={{ fontSize: 11, opacity: 0.6, wordBreak: "break-all" }}>{s.restricted ? "Private address (not shared)" : s.url ?? s.host}</div>
              {s.agent && (
                <div data-testid="br-agent-chip" style={{ fontSize: 11 }}>
                  {s.agent.mode === "drive" ? "Drive" : "Observe"} · {s.agent.name}
                </div>
              )}
              <button
                type="button"
                data-testid="br-shot"
                disabled={!online || busy}
                style={btn}
                onClick={() =>
                  void wrap(async () => {
                    const id = await commander.send("browser.screenshot", { sessionId: s.sessionId }, { label: `Screenshot ${s.host}` });
                    setShotCmd((m) => ({ ...m, [s.sessionId]: id }));
                  })
                }
              >
                {shot ? "See another screenshot" : "See screenshot"}
              </button>
              {agentsAllowed && (
                <button
                  type="button"
                  data-testid="br-robot"
                  aria-label="Observe or drive with an agent"
                  title="Observe & Drive"
                  style={btn}
                  disabled={!online}
                  onClick={() =>
                    setRobot(robot?.id === s.sessionId ? null : { id: s.sessionId, agent: s.agent?.pubkey ?? "", mode: s.agent?.mode ?? "observe", channel: channels[0]?.id ?? "" })
                  }
                >
                  🤖
                </button>
              )}
              <button type="button" data-testid="br-remove" style={btn} disabled={!online} onClick={() => setConfirming({ type: "close", id: s.sessionId, host: s.title || s.host })}>
                Remove…
              </button>
              {agentsAllowed && robot?.id === s.sessionId && (
                <div data-testid="br-robot-panel" style={{ border: "1px solid #8884", borderRadius: 8, padding: 8, fontSize: 12, marginTop: 6 }}>
                  <label>
                    Agent{" "}
                    <select data-testid="br-robot-agent" value={robot.agent} onChange={(e) => setRobot({ ...robot, agent: e.target.value })}>
                      <option value="">Choose…</option>
                      {(bridge.snapshot?.agents ?? []).map((a) => (
                        <option key={a.pubkey} value={a.pubkey}>
                          {a.name}
                        </option>
                      ))}
                    </select>
                  </label>{" "}
                  <label>
                    Channel{" "}
                    <select data-testid="br-robot-channel" value={robot.channel} onChange={(e) => setRobot({ ...robot, channel: e.target.value })}>
                      {channels.map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <div style={{ marginTop: 6 }}>
                    {(["off", "observe", "drive"] as const).map((m) => (
                      <label key={m} style={{ marginRight: 10 }}>
                        <input type="radio" name={`mode-${s.sessionId}`} checked={robot.mode === m} onChange={() => setRobot({ ...robot, mode: m })} />{" "}
                        {m === "off" ? "Off" : m === "observe" ? "Observe (view only)" : "Drive (agent can click and type)"}
                      </label>
                    ))}
                  </div>
                  <button
                    type="button"
                    data-testid="br-robot-apply"
                    style={{ ...btn, marginTop: 6 }}
                    disabled={robot.mode !== "off" && (!robot.agent || !robot.channel)}
                    onClick={() =>
                      void wrap(async () => {
                        await commander.send(
                          "browser.agent",
                          robot.mode === "off" ? { sessionId: s.sessionId, mode: "off" } : { sessionId: s.sessionId, mode: robot.mode, agentPubkey: robot.agent, channelId: robot.channel },
                          { confirmed: true, label: `${robot.mode === "off" ? "Stop agent on" : robot.mode === "drive" ? "Drive" : "Observe"} ${s.host}` },
                        );
                        setRobot(null);
                      })
                    }
                  >
                    {robot.mode === "drive" ? "Let the agent drive" : "Apply"}
                  </button>
                </div>
              )}
              {st && st !== "ok" && (
                <span data-testid="br-shot-status" style={{ fontSize: 11 }}>
                  {st === "pending" ? "requesting…" : st === "accepted" ? "taking screenshot…" : st === "timeout" ? "desktop did not respond" : `failed${r?.message ? `: ${r.message}` : ""}`}
                </span>
              )}
              {shot && (
                <div style={{ marginTop: 6 }}>
                  <img data-testid="br-shot-img" src={shot.src} alt={`Screenshot of ${s.host}`} style={{ maxWidth: "100%", border: "1px solid #8884", borderRadius: 6 }} />
                  <div style={{ fontSize: 11, opacity: 0.6 }}>Taken {new Date(shot.ts * 1000).toLocaleTimeString()}</div>
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {recent.length > 0 && (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, fontSize: 11, opacity: 0.7 }}>
          {recent.map((c) => {
            const st = commander.statusOf(c.id, nowS());
            const r = bridge.results.get(c.id);
            return (
              <li key={c.id}>
                {c.label}: {st === "ok" ? "done" : st === "error" ? `failed${r?.message ? `: ${r.message}` : ""}` : st === "timeout" ? "no response" : "working…"}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
