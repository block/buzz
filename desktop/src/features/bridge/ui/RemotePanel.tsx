// Slice B web UI: lists the member's sharing desktops (from the signed heartbeat) and opens a terminal tab.
import * as React from "react";

import { describeRemote } from "../remoteCopy";
import { RemoteConnection, RemoteError, type RemoteSession, type SignJson, type TerminalHandle } from "../remoteClient";
import { KEYS, TermBuffer } from "../termBuffer";
import { attachHandoffToTerminal, takePendingTermHandoff, peekPendingTermHandoff, type PendingTermHandoff } from "../webTermHandoff";
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
  // Effective Buzz Term setting on the desktop. termHost undefined = an older desktop that doesn't advertise it (legacy behaviour).
  const termHost = info?.termHost;
  const herdrMode = termHost === "herdr";
  const [session, setSession] = React.useState<string | null>(null);
  const defaultSession = herdrMode ? (info?.termSession ?? "") : "";
  const chosenSession = session ?? defaultSession;
  const [open, setOpen] = React.useState(false);
  const [line, setLine] = React.useState("");
  const screenRef = React.useRef<HTMLPreElement>(null);

  // A "Pass thread context to Buzz Term" handoff from the web: open the terminal by itself and start the session in it.
  const autoRef = React.useRef<((h: PendingTermHandoff) => void) | null>(null);
  const autoReady = !!info && info.status === "on" && info.terminals && !open;
  React.useEffect(() => {
    if (!autoReady || !peekPendingTermHandoff()) return;
    const h = takePendingTermHandoff();
    if (h) autoRef.current?.(h);
  }, [autoReady]);
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
  const openTerm = (handoff?: PendingTermHandoff) =>
    guard("terminal", async () => {
      const c = await ensure();
      bufRef.current = new TermBuffer();
      const dec = new TextDecoder();
      const t = await c.openTerminal(
        {
          cols: 80,
          rows: 24,
          session: herdrMode || termHost === undefined ? chosenSession || undefined : undefined,
          // the host is chosen per launch: what the desktop's Buzz Term setting says it uses (the desktop never grants more than its own setting)
          ...(termHost ? { host: herdrMode ? ("herdr" as const) : ("shell" as const) } : {}),
        },
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
      // the desktop's Settings → Buzz Term → Scrollback decides how much history this terminal keeps
      if (t.scrollback) bufRef.current.maxLines = t.scrollback;
      setOpen(true);
      if (handoff) attachHandoffToTerminal(t, handoff);
    });
  autoRef.current = (h) => void openTerm(h);
  const send = (s: string) => termRef.current?.write(s);
  const btn = { fontSize: 12, padding: "4px 8px", marginRight: 4 } as const;

  return (
    <div data-testid="remote-panel" style={{ marginTop: 16, borderTop: "1px solid #8884", paddingTop: 8 }}>
      <h3 style={{ fontSize: 14, margin: "0 0 6px" }}>Buzz term on {info.name}</h3>
      {termHost && (
        <p data-testid="remote-term-setting" style={{ fontSize: 12, margin: "0 0 6px", opacity: 0.85 }}>
          {herdrMode
            ? `Using herdr: ${info.termSession ? `session ${info.termSession}` : "the default session"} (Settings → Buzz Term → Session host on ${info.name}).`
            : `Using a normal shell on ${info.name} (Settings → Buzz Term → Session host).`}
        </p>
      )}
      {d.message && (
        <p data-testid="remote-message" style={{ fontSize: 12, margin: "0 0 6px", color: d.problem === "none" ? undefined : "#c0392b" }}>
          {d.message}
        </p>
      )}
      {info.status === "on" && !info.terminals && (
        <p data-testid="remote-terminals-off" style={{ fontSize: 12 }}>"Allow remote terminal" is off on {info.name}.</p>
      )}
      {usable && info.status === "on" && info.terminals && !open && (
        <div style={{ display: "grid", gap: 6 }}>
          {(herdrMode || termHost === undefined) && (
          <div>
            <button type="button" data-testid="remote-sessions" style={btn} disabled={!!busy} onClick={() => void loadSessions()}>
              List herdr sessions
            </button>
            {(sessions.length > 0 || herdrMode) && (
              <select data-testid="remote-session-select" value={chosenSession} onChange={(e) => setSession(e.target.value)} style={{ fontSize: 12 }}>
                {herdrMode ? (
                  <option value={defaultSession}>{defaultSession ? `herdr: ${defaultSession} (default session)` : "herdr: default session"}</option>
                ) : (
                  <option value="">Plain shell (new)</option>
                )}
                {sessions.filter((x) => !herdrMode || x.name !== defaultSession).map((s) => (
                  <option key={s.name} value={s.name}>
                    herdr: {s.name} ({s.status})
                  </option>
                ))}
              </select>
            )}
          </div>
          )}
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
