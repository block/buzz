// Self-contained mock NIP-46 remote signer + a tiny in-process NIP-01 websocket relay for the signer channel.
// The user's secret key (userSkHex) stays in this Node process, like a real remote signer; the page only sees signed events.
import { readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  finalizeEvent,
  getPublicKey,
  nip04,
  nip44,
  verifyEvent,
  generateSecretKey,
} from "nostr-tools";
import { bytesToHex, hexToBytes } from "nostr-tools/utils";

// `ws` is only a transitive dev dependency (via jsdom); resolve it directly or from the pnpm store.
async function loadWs() {
  try {
    return await import("ws");
  } catch {}
  const store = join(
    dirname(fileURLToPath(import.meta.url)),
    "../../../node_modules/.pnpm",
  );
  const dir = readdirSync(store).find((d) => d.startsWith("ws@"));
  if (!dir)
    throw new Error("ws package not found; add `ws` as a devDependency");
  return import(
    pathToFileURL(join(store, dir, "node_modules/ws/wrapper.mjs")).href
  );
}

export async function startMockSigner({
  userSkHex,
  port = 0,
  secret = "test-secret-" + Math.random().toString(36).slice(2),
  silent = false, // never answer (timeout test)
  authUrl = null, // if set, first answer connect with auth_url
  useNip04 = false,
  latencyMs = 0, // artificial per-request delay before answering
  dropRate = 0, // fraction of nip44_encrypt/decrypt/sign_event requests silently dropped (0..1)
  dropMethods = ["nip44_decrypt", "nip44_encrypt"],
  jitterMs = 0, // extra random 0..jitterMs added to latencyMs
  serial = false, // process requests one at a time (like a single backgrounded signer app)
} = {}) {
  const userSk = hexToBytes(userSkHex);
  const userPk = getPublicKey(userSk);
  const signerSk = generateSecretKey(); // transport identity of the signer (differs from the user key, like nsec.app)
  const signerPk = getPublicKey(signerSk);
  const subs = new Set(); // {ws, id, filters}
  const log = { requests: [], signedKinds: [], unauthorized: 0, timeline: [] };
  const state = { silent, authUrl, latencyMs, dropRate, dropMethods, jitterMs, serial, dropSignKinds: new Map() }; // dropSignKinds: kind -> remaining count to silently drop
  const { WebSocketServer } = await loadWs();
  const wss = new WebSocketServer({ port });
  await new Promise((r) => wss.on("listening", r));
  const realPort = wss.address().port;
  const url = `ws://127.0.0.1:${realPort}`;
  const connectedClients = new Set();

  const matches = (f, e) =>
    (!f.kinds || f.kinds.includes(e.kind)) &&
    (!f["#p"] || e.tags.some((t) => t[0] === "p" && f["#p"].includes(t[1])));
  const stored = []; // like a real relay: keep events so a client that was away (reload/suspend) can fetch them via `since`
  function broadcast(e) {
    stored.push(e);
    for (const s of subs)
      if (s.filters.some((f) => matches(f, e)) && s.ws.readyState === 1)
        s.ws.send(JSON.stringify(["EVENT", s.id, e]));
  }
  const enc = (peer, text) =>
    useNip04
      ? nip04.encrypt(signerSk, peer, text)
      : nip44.v2.encrypt(
          text,
          nip44.v2.utils.getConversationKey(signerSk, peer),
        );
  const dec = (peer, ct) =>
    ct.includes("?iv=")
      ? nip04.decrypt(signerSk, peer, ct)
      : nip44.v2.decrypt(ct, nip44.v2.utils.getConversationKey(signerSk, peer));
  async function reply(clientPk, payload) {
    const ev = finalizeEvent(
      {
        kind: 24133,
        created_at: Math.floor(Date.now() / 1000),
        tags: [["p", clientPk]],
        content: await enc(clientPk, JSON.stringify(payload)),
      },
      signerSk,
    );
    broadcast(ev);
  }
  async function handle(ev) {
    if (!verifyEvent(ev)) return;
    if (state.silent) {
      log.dropped = (log.dropped ?? 0) + 1;
      return;
    }
    let req;
    try {
      req = JSON.parse(await dec(ev.pubkey, ev.content));
    } catch {
      return;
    }
    log.requests.push(req.method);
    log.timeline.push({ t: Date.now(), method: req.method, kind: req.method === "sign_event" ? (() => { try { return JSON.parse(req.params[0]).kind; } catch { return null; } })() : undefined });
    if (state.dropRate > 0 && state.dropMethods.includes(req.method) && Math.random() < state.dropRate) {
      log.dropped = (log.dropped ?? 0) + 1;
      return;
    }
    if (req.method === "sign_event") {
      let k;
      try { k = JSON.parse(req.params[0]).kind; } catch {}
      const left = state.dropSignKinds.get(k) ?? 0;
      if (left > 0) {
        state.dropSignKinds.set(k, left - 1);
        log.dropped = (log.dropped ?? 0) + 1;
        return;
      }
    }
    const lat = state.latencyMs + Math.random() * state.jitterMs;
    if (lat > 0) await new Promise((r) => setTimeout(r, lat));
    const [a, b] = req.params ?? [];
    const ok = (result) => reply(ev.pubkey, { id: req.id, result });
    const err = (error) => reply(ev.pubkey, { id: req.id, result: "", error });
    if (state.revoked && req.method !== "connect") return err("unauthorized: session revoked");
    switch (req.method) {
      case "connect":
        if (secret && b !== secret) return err("invalid secret");
        connectedClients.add(ev.pubkey);
        state.revoked = false; // a fresh pairing restores service
        if (state.authUrl && !state._authed) {
          state._authed = true;
          await reply(ev.pubkey, {
            id: req.id,
            result: "auth_url",
            error: state.authUrl,
          });
          setTimeout(() => ok("ack"), 400);
          return;
        }
        return ok("ack");
      case "ping":
        return ok("pong");
      case "get_public_key":
        if (!connectedClients.has(ev.pubkey)) return err("not connected");
        return ok(userPk);
      case "sign_event": {
        if (!connectedClients.has(ev.pubkey)) {
          log.unauthorized++;
          return err("not connected");
        }
        const d = JSON.parse(a);
        const signed = finalizeEvent(
          {
            kind: d.kind,
            content: d.content,
            tags: d.tags,
            created_at: d.created_at,
          },
          userSk,
        );
        log.signedKinds.push(signed.kind);
        return ok(JSON.stringify(signed));
      }
      case "nip44_encrypt":
        return ok(
          nip44.v2.encrypt(b, nip44.v2.utils.getConversationKey(userSk, a)),
        );
      case "nip44_decrypt":
        return ok(
          nip44.v2.decrypt(b, nip44.v2.utils.getConversationKey(userSk, a)),
        );
      default:
        return err("unsupported method " + req.method);
    }
  }
  let chain = Promise.resolve(); // `serial`: the signer handles one request at a time (a backgrounded phone app)
  const enqueue = (e) => {
    if (state.serial) chain = chain.then(() => handle(e)).catch(() => {});
    else void handle(e);
  };
  wss.on("connection", (ws) => {
    ws.on("message", (raw) => {
      let m;
      try {
        m = JSON.parse(String(raw));
      } catch {
        return;
      }
      if (m[0] === "REQ") {
        const filters = m.slice(2);
        subs.add({ ws, id: m[1], filters });
        log.reqs = (log.reqs ?? 0) + 1;
        for (const e of stored)
          if (
            filters.some(
              (f) => matches(f, e) && (f.since === undefined || e.created_at >= f.since),
            )
          )
            ws.send(JSON.stringify(["EVENT", m[1], e]));
        ws.send(JSON.stringify(["EOSE", m[1]]));
      } else if (m[0] === "CLOSE") {
        for (const s of subs) if (s.ws === ws && s.id === m[1]) subs.delete(s);
      } else if (m[0] === "EVENT") {
        const e = m[1];
        ws.send(JSON.stringify(["OK", e.id, true, ""]));
        broadcast(e);
        if (e.tags.some((t) => t[0] === "p" && t[1] === signerPk))
          enqueue(e);
      }
    });
    ws.on("close", () => {
      for (const s of subs) if (s.ws === ws) subs.delete(s);
    });
  });

  return {
    url,
    userPk,
    signerPk,
    secret,
    log,
    state,
    bunkerUrl: (extra = "") =>
      `bunker://${signerPk}?relay=${encodeURIComponent(url)}&secret=${secret}${extra}`,
    /** Emulate scanning a nostrconnect:// URI: answer on the URI's relay with {result: secret}. */
    async scan(uri, { result } = {}) {
      const u = new URL(uri);
      const clientPk = u.hostname;
      const sec = u.searchParams.get("secret");
      connectedClients.add(clientPk);
      await reply(clientPk, { id: crypto.randomUUID(), result: result ?? sec });
    },
    /** Signer app closed / offline: requests are received but never answered. */
    setOffline(on) {
      state.silent = !!on;
    },
    /** Primal connection removed: every client must pair again (answers "not connected"/"unauthorized"). */
    revokeClients() {
      connectedClients.clear();
      state.revoked = true;
    },
    setLatency(ms) {
      state.latencyMs = ms;
    },
    dropAllSockets() {
      for (const c of wss.clients) c.terminate();
    },
    async close() {
      for (const c of wss.clients) c.terminate();
      await new Promise((r) => wss.close(r));
    },
  };
}
