// Slice B web UI: lists the member's sharing desktops (from the signed heartbeat) and opens a terminal tab.
import * as React from "react";

import { describeRemote } from "../remoteCopy";
import { RemoteConnection, RemoteError, type RemoteSession, type SignJson, type TerminalHandle } from "../remoteClient";
import { KEYS, TermBuffer } from "../termBuffer";
import type { BridgeState } from "../webClient";

export function RemotePanel(props: { bridge: BridgeState; signJson: SignJson }) {
  const { bridge, signJson } = props;
  const info = bridge.heartbeat?.remote;
  const [failure, setFailure] = React.useState<string | undefined>();
  const [busy, setBusy] = React.useState<string | null>(null);
  const connRef = React.useRef<RemoteConnection | null>(null);
  const termRef = React.useRef<TerminalHandle | null>(null);
  const bufRef = React.useRef(new TermBuffer());
  const [, force] = React.useReducer((x: number) => x + 1, 0);
  const [sessions, setSessions] = React.useState<RemoteSession[]>([]);
  const [session, setSession] = React.useState("");
  const [open, setOpen] = React.useState(false);
  const [line, setLine] = React.useState("");
  const screenRef = React.useRef<HTMLPreElement>(null);

  React.useEffect(
    () => () => {
      termRef.current?.close();
      connRef.current?.close();
    },
    [],
  );
  React.useEffect(() => {
    const el = screenRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  });
  if (!info) return null;
  const d = describeRemote(bridge.presence, info, failure);
  const usable = d.problem === "none" || d.problem === "terminals_off" || d.problem === "tailscale_device" || d.problem === "declined" || d.problem === "other";

  const ensure = async () => {
    if (connRef.current?.ready) return connRef.current;
    const c = new RemoteConnection(info, signJson);
    c.onClose = () => {
      connRef.current = null;
      setOpen(false);
      termRef.current = null;
      force();
    };
    await c.connect();
    connRef.current = c;
    return c;
  };
  const guard = async (what: string, fn: () => Promise<void>) => {
    setBusy(what);
    setFailure(undefined);
    try {
      await fn();
    } catch (e) {
      const code = e instanceof RemoteError ? e.code : /reject|denied|declin/i.test(String(e)) ? "declined" : "other";
      setFailure(code);
    }
    setBusy(null);
  };
  const loadSessions = () =>
    guard("sessions", async () => {
      const c = await ensure();
      setSessions(await c.listSessions());
    });
  const openTerm = () =>
    guard("terminal", async () => {
      const c = await ensure();
      bufRef.current = new TermBuffer();
      const dec = new TextDecoder();
      const t = await c.openTerminal(
        { cols: 80, rows: 24, session: session || undefined },
        (bytes) => {
          bufRef.current.write(dec.decode(bytes, { stream: true }));
          force();
        },
        () => {
          termRef.current = null;
          setOpen(false);
          force();
        },
      );
      termRef.current = t;
      setOpen(true);
    });
  const send = (s: string) => termRef.current?.write(s);
  const btn = { fontSize: 12, padding: "4px 8px", marginRight: 4 } as const;

  return (
    <div data-testid="remote-panel" style={{ marginTop: 16, borderTop: "1px solid #8884", paddingTop: 8 }}>
      <h3 style={{ fontSize: 14, margin: "0 0 6px" }}>Terminal on {info.name}</h3>
      {d.message && (
        <p data-testid="remote-message" style={{ fontSize: 12, margin: "0 0 6px", color: d.problem === "none" ? undefined : "#c0392b" }}>
          {d.message}
        </p>
      )}
      {info.status === "on" && !info.terminals && (
        <p data-testid="remote-terminals-off" style={{ fontSize: 12 }}>Remote terminals are off on {info.name}.</p>
      )}
      {usable && info.status === "on" && info.terminals && !open && (
        <div style={{ display: "grid", gap: 6 }}>
          <div>
            <button type="button" data-testid="remote-sessions" style={btn} disabled={!!busy} onClick={() => void loadSessions()}>
              List herdr sessions
            </button>
            {sessions.length > 0 && (
              <select data-testid="remote-session-select" value={session} onChange={(e) => setSession(e.target.value)} style={{ fontSize: 12 }}>
                <option value="">Plain shell (new)</option>
                {sessions.map((s) => (
                  <option key={s.name} value={s.name}>
                    herdr: {s.name} ({s.status})
                  </option>
                ))}
              </select>
            )}
          </div>
          <div>
            <button type="button" data-testid="remote-open" style={btn} disabled={!!busy} onClick={() => void openTerm()}>
              {busy === "terminal" ? "Waiting for approval…" : "Open terminal"}
            </button>
            <span style={{ fontSize: 11, opacity: 0.6 }}>Asks your signer to approve each launch.</span>
          </div>
        </div>
      )}
      {open && (
        <div data-testid="remote-terminal">
          <pre
            ref={screenRef}
            data-testid="remote-screen"
            style={{ background: "#111", color: "#ddd", fontSize: 12, padding: 8, height: 260, overflow: "auto", margin: 0, whiteSpace: "pre-wrap", wordBreak: "break-all" }}
          >
            {bufRef.current.text()}
          </pre>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              send(line + KEYS.enter);
              setLine("");
            }}
            style={{ display: "flex", gap: 4, marginTop: 4 }}
          >
            <input
              data-testid="remote-input"
              value={line}
              onChange={(e) => setLine(e.target.value)}
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              style={{ flex: 1, fontSize: 14, fontFamily: "monospace" }}
              placeholder="type a command, Enter to send"
            />
          </form>
          <div style={{ marginTop: 4 }}>
            {(["tab", "esc", "ctrlC", "ctrlD", "up", "down", "left", "right"] as const).map((k) => (
              <button key={k} type="button" style={btn} onClick={() => send(KEYS[k])}>
                {k === "ctrlC" ? "Ctrl-C" : k === "ctrlD" ? "Ctrl-D" : k}
              </button>
            ))}
            <button type="button" style={btn} data-testid="remote-close" onClick={() => termRef.current?.close()}>
              Close terminal
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
