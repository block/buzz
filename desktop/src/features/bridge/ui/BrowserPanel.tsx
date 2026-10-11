// Slice R web UI: "Open on desktop" (confirmed) + per-session "See screenshot" (on demand, view-only) + close.
import * as React from "react";

import { nowS } from "../protocol";
import type { BridgeCommander } from "../commander";
import type { BridgeState } from "../webClient";

export function BrowserPanel(props: { commander: BridgeCommander; bridge: BridgeState }) {
  const { commander, bridge } = props;
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
      <h3 style={{ fontSize: 14, margin: "0 0 6px" }}>Desktop browser</h3>
      <p style={{ fontSize: 11, opacity: 0.6, margin: "0 0 6px" }}>
        View-only. Opens a link in your desktop's built-in browser; screenshots are taken only when you ask.
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
          Open on desktop…
        </button>
      </div>
      {confirming && (
        <div data-testid="br-confirm" role="alertdialog" style={{ border: "1px solid #c0392b", borderRadius: 8, padding: 8, fontSize: 12, marginTop: 6 }}>
          {confirming.type === "open"
            ? `Open ${url.trim().slice(0, 200)} on your desktop? It will show as a remotely visible browser session there.`
            : `Stop sharing ${confirming.host} and remove it from your desktop?`}
          <div style={{ marginTop: 6 }}>
            <button
              type="button"
              data-testid="br-confirm-yes"
              style={btn}
              onClick={() =>
                void wrap(async () => {
                  if (confirming.type === "open") {
                    await commander.send("browser.open", { url: url.trim() }, { confirmed: true, label: `Open ${url.trim().slice(0, 40)}` });
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
              <div style={{ fontSize: 13 }}>{s.title || s.host}</div>
              <div style={{ fontSize: 11, opacity: 0.6 }}>{s.host}</div>
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
              <button type="button" style={btn} disabled={!online} onClick={() => setConfirming({ type: "close", id: s.sessionId, host: s.host })}>
                Close…
              </button>
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
