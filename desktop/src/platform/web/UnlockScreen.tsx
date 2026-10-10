// Pre-app unlock gate for the web build. Standalone (inline styles) so it renders before the app CSS loads.
import { QRCodeSVG } from "qrcode.react";
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  adopt,
  detectNip07,
  signIn,
  signOutSigner,
  getSession,
} from "./signer";
import {
  DEFAULT_SIGNER_RELAYS,
  type NostrConnectFlow,
  beginNostrConnect,
  connectBunker,
} from "./signer/nip46";
import { dbgEnabled, nip46Debug } from "./signer/nip46Debug";
import { peekPendingConnect } from "./signer/nip46Store";

const isIos = () =>
  /iPad|iPhone|iPod/.test(navigator.userAgent) ||
  (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
const input: React.CSSProperties = {
  padding: "12px 12px",
  borderRadius: 10,
  border: "1px solid #444",
  background: "#111",
  color: "#f5f5f5",
  fontSize: 16, // 16px: iOS Safari zooms on smaller inputs
  width: "100%",
  boxSizing: "border-box",
};
const ghost: React.CSSProperties = {
  padding: "12px 16px",
  borderRadius: 10,
  border: "1px solid #444",
  background: "transparent",
  color: "#ddd",
  fontWeight: 600,
  fontSize: 15,
  cursor: "pointer",
  textAlign: "center",
  textDecoration: "none",
};
const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));

function DebugLine() {
  const [, tick] = useState(0);
  useEffect(() => nip46Debug.subscribe(() => tick((n) => n + 1)), []);
  const d = nip46Debug.get();
  const urls = Object.keys(d.relays);
  const open = urls.filter((u) => d.relays[u]).length;
  return (
    <p
      style={{ ...note, fontFamily: "ui-monospace, monospace", fontSize: 11 }}
      data-testid="web-unlock-debug"
    >
      signer relays: connected {open}/{urls.length}, last event:{" "}
      {d.lastEvent ?? "none"}
      {d.note ? ` · ${d.note}` : ""}
    </p>
  );
}

function Nip46Panel({ onDone }: { onDone: (pubkey: string) => void }) {
  const [bunker, setBunker] = useState("");
  const [relay, setRelay] = useState(
    new URLSearchParams(location.search).get("nip46relay") ||
      DEFAULT_SIGNER_RELAYS.join(", "),
  );
  const [uri, setUri] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | "bunker" | "connect">(
    peekPendingConnect() ? "connect" : null,
  );
  const [resumed, setResumed] = useState<boolean>(!!peekPendingConnect());
  const [err, setErr] = useState<string | null>(null);
  const flowRef = useRef<NostrConnectFlow | null>(null);
  // Unmount: stop listening but KEEP the pending attempt (a reload must be able to resume it).
  useEffect(() => () => flowRef.current?.detach(), []);

  const attach = async (flow: NostrConnectFlow) => {
    flowRef.current = flow;
    setResumed(flow.resumed);
    setUri(flow.uri);
    setBusy("connect");
    try {
      onDone(await adopt(await flow.connected));
    } catch (e) {
      if (flowRef.current !== flow) return; // detached/replaced
      setErr(errText(e));
      setUri(null);
      setBusy(null);
      setResumed(false);
    }
  };
  // Auto-resume a pending nostrconnect attempt after a reload / tab discard / return from the signer app.
  useEffect(() => {
    if (!peekPendingConnect()) return;
    let live = true;
    void beginNostrConnect(undefined, { resumeOnly: true }).then((flow) => {
      if (!live) return flow?.detach();
      if (flow) void attach(flow);
      else {
        setBusy(null);
        setResumed(false);
      }
    });
    return () => {
      live = false;
    };
    // biome-ignore lint/correctness/useExhaustiveDependencies: run once on mount
  }, []);

  const doBunker = async () => {
    setBusy("bunker");
    setErr(null);
    try {
      onDone(await adopt(await connectBunker(bunker)));
    } catch (e) {
      setErr(errText(e));
      setBusy(null);
    }
  };
  const doConnect = async () => {
    setErr(null);
    setBusy("connect");
    const flow = await beginNostrConnect(
      relay
        .split(/[\s,]+/)
        .filter(Boolean)
        .map((r) => r.trim()),
    );
    if (flow) void attach(flow);
  };
  return (
    <div
      style={{ display: "grid", gap: 10 }}
      data-testid="web-unlock-nip46-panel"
    >
      <input
        style={input}
        placeholder="bunker://…"
        value={bunker}
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        onChange={(e) => setBunker(e.target.value)}
        data-testid="web-unlock-bunker-input"
      />
      <button
        type="button"
        style={busy || !bunker.trim() ? btnOff : btn}
        disabled={!!busy || !bunker.trim()}
        onClick={doBunker}
        data-testid="web-unlock-bunker-go"
      >
        {busy === "bunker"
          ? "Waiting for your signer…"
          : "Connect with bunker URL"}
      </button>
      {busy === "connect" && (
        <p
          role="status"
          style={{ ...note, color: "#e6b450", fontSize: 15 }}
          data-testid="web-unlock-finishing"
        >
          {resumed ? "Finishing sign-in…" : "Waiting for your signer…"}
        </p>
      )}
      <p style={note}>— or connect by scanning / opening a link —</p>
      {!uri && (
        <>
          <input
            style={input}
            value={relay}
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            onChange={(e) => setRelay(e.target.value)}
            aria-label="Signer relay"
            data-testid="web-unlock-signer-relay"
          />
          <button
            type="button"
            style={busy ? btnOff : ghost}
            disabled={!!busy}
            onClick={doConnect}
            data-testid="web-unlock-nostrconnect"
          >
            Show connect QR / link
          </button>
        </>
      )}
      {uri && (
        <div
          style={{ display: "grid", gap: 10, justifyItems: "center" }}
          data-testid="web-unlock-nostrconnect-panel"
        >
          <div style={{ background: "#fff", padding: 10, borderRadius: 10 }}>
            <QRCodeSVG value={uri} size={200} />
          </div>
          <a
            href={uri}
            style={{
              ...btn,
              textDecoration: "none",
              textAlign: "center",
              width: "100%",
              boxSizing: "border-box",
            }}
            data-testid="web-unlock-open-signer"
          >
            Open in signer app
          </a>
          <input
            style={input}
            readOnly
            value={uri}
            onFocus={(e) => e.currentTarget.select()}
            data-testid="web-unlock-nostrconnect-uri"
          />
          <p style={note}>
            Approve in your signer app, then come back here. This page will
            finish signing in on its own (valid 10 min).
          </p>
          <button
            type="button"
            style={ghost}
            data-testid="web-unlock-cancel"
            onClick={() => {
              flowRef.current?.cancel();
              flowRef.current = null;
              setUri(null);
              setBusy(null);
              setResumed(false);
            }}
          >
            Cancel
          </button>
        </div>
      )}
      {dbgEnabled() && <DebugLine />}
      {err && (
        <p
          role="alert"
          style={{ color: "#ff6b6b" }}
          data-testid="web-unlock-nip46-error"
        >
          {err}
        </p>
      )}
    </div>
  );
}

const box: React.CSSProperties = {
  minHeight: "100dvh",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "#000",
  color: "#f5f5f5",
  font: "15px/1.5 Inter, system-ui, sans-serif",
  padding:
    "env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom) env(safe-area-inset-left)",
};
const card: React.CSSProperties = {
  width: "min(420px, 92vw)",
  display: "grid",
  gap: 14,
};
const btn: React.CSSProperties = {
  padding: "12px 16px",
  borderRadius: 10,
  border: "1px solid #444",
  background: "#fff",
  color: "#000",
  fontWeight: 600,
  fontSize: 15,
  cursor: "pointer",
};
const btnOff: React.CSSProperties = {
  ...btn,
  background: "#111",
  color: "#888",
  cursor: "not-allowed",
};
const note: React.CSSProperties = { color: "#9a9a9a", fontSize: 13 };

function Unlock({ onDone }: { onDone: (pubkey: string) => void }) {
  const [ext, setExt] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    void detectNip07().then(setExt);
  }, []);
  const go = async () => {
    setBusy(true);
    setErr(null);
    try {
      onDone(await signIn("nip07"));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };
  return (
    <div style={box}>
      <div style={card} data-testid="web-unlock">
        <h1 style={{ fontSize: 24, margin: 0 }}>Sign in to Buzz</h1>
        <p style={note}>
          Your private key never enters this page. Sign-in and every message is
          signed by your browser extension or remote signer.
        </p>
        {ext === null && <p style={note}>Looking for a Nostr extension…</p>}
        {ext === true && (
          <button
            type="button"
            style={btn}
            disabled={busy}
            onClick={go}
            data-testid="web-unlock-nip07"
          >
            {busy ? "Waiting for extension…" : "Sign in with browser extension"}
          </button>
        )}
        {ext === false && !isIos() && (
          <div
            data-testid="web-unlock-no-extension"
            style={{ display: "grid", gap: 8 }}
          >
            <button type="button" style={btnOff} disabled>
              Sign in with browser extension
            </button>
            <p style={{ ...note, color: "#e6b450" }}>
              No NIP-07 extension (window.nostr) found. Install one such as
              Alby, nos2x or Nostr Connect, then reload this page.
            </p>
            <button
              type="button"
              style={{ ...btn, background: "transparent", color: "#ddd" }}
              onClick={() => {
                setExt(null);
                void detectNip07(3000).then(setExt);
              }}
            >
              Check again
            </button>
          </div>
        )}
        <div
          style={{
            borderTop: ext ? "1px solid #222" : undefined,
            paddingTop: 12,
            display: "grid",
            gap: 8,
          }}
        >
          <strong data-testid="web-unlock-nip46">Remote signer (NIP-46)</strong>
          <p style={note}>
            Sign with nsec.app, Amber, Primal or any bunker. Your key stays in
            that app.
            {isIos() && " Extensions don't exist on iPhone/iPad, so use this."}
          </p>
          <Nip46Panel onDone={onDone} />
        </div>
        {getSession() && (
          <button
            type="button"
            style={ghost}
            data-testid="web-unlock-forget"
            onClick={async () => {
              await signOutSigner();
              location.reload();
            }}
          >
            Forget saved signer
          </button>
        )}
        {err && (
          <p role="alert" style={{ color: "#ff6b6b" }}>
            {err}
          </p>
        )}
      </div>
    </div>
  );
}

export function showUnlock(): Promise<string> {
  const el = document.getElementById("root") as HTMLElement;
  const root = createRoot(el);
  return new Promise((resolve) => {
    root.render(
      <Unlock
        onDone={(pk) => {
          root.unmount();
          resolve(pk);
        }}
      />,
    );
  });
}
