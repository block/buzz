// NIP-46 remote signer client. The user's secret key NEVER enters this page: it lives in the remote signer
// (nsec.app, Amber, Primal, a bunker...). We hold only an ephemeral *client* keypair that identifies this browser to
// the signer over the signer's own relay(s). Requests/responses are kind 24133 events encrypted with NIP-44
// (NIP-04 only if the signer answers in NIP-04).
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
  nip04,
  nip44,
  verifyEvent,
} from "nostr-tools";
import { bytesToHex, hexToBytes } from "nostr-tools/utils";
import {
  PENDING_TTL_MS,
  type PendingConnect,
  clearNip46Session,
  clearPendingConnect,
  loadNip46Session,
  loadPendingConnect,
  saveNip46Session,
  savePendingConnect,
} from "./nip46Store";
import { nip46Debug, nlog, short } from "./nip46Debug";
import { signerHealth } from "./signerHealth";
import { signerStatus } from "./signerStatus";
import {
  type SignedEvent,
  type Signer,
  SignerUnavailableError,
  type UnsignedEvent,
} from "./types";

export const NIP46_AVAILABLE = true;
export const NIP46_KIND = 24133;
// Primal's signer answers on its own relay; nsec.app on relay.nsec.app. Listing several lets whichever the signer uses work.
export const DEFAULT_SIGNER_RELAYS = [
  "wss://relay.nsec.app",
  "wss://relay.primal.net",
  "wss://nrs.primal.net",
  "wss://relay.damus.io",
];
const REQUEST_TIMEOUT_MS = 30_000;
const CONNECT_TIMEOUT_MS = 120_000; // bunker connect / get_public_key
const AUTH_URL_TIMEOUT_MS = 180_000; // user is approving in a browser tab / signer app
const HANDSHAKE_METHODS = new Set(["connect", "get_public_key"]);

// Plain methods plus explicit per-kind sign_event grants: some signers (Primal) only pre-approve kinds that are named.
// Kinds Buzz web signs: auth 22242, HTTP auth 27235, profile 0, notes 1, deletes 5, reactions 7, chat 9, join/leave 9021/9022,
// community profile 9033, reports 1984, presence/typing 20001/20002, app data 30078 (settings/read state), emoji 30030,
// reminders 30300, status 30315, community pins/bots/sections 30623-30625, Blossom media auth 24242 (relay avatars/images),
// forum/workflow kinds 40002/40003/40008/45001/45003.
export const SIGN_KINDS = [22242, 27235, 9, 7, 0, 1, 5, 9021, 9022, 9033, 1984, 20001, 20002, 30078, 30030, 30300, 30315, 30623, 30624, 30625, 24242, 40002, 40003, 40008, 45001, 45003];
// The kinds named in the nostrconnect:// / connect perms. Frozen at the set Primal iOS is known to accept: adding kinds it
// doesn't recognise (24242 Blossom auth, 40002/40003/40008/45001/45003 forum/workflow — #244) left new PWA installs stuck at
// "Waiting for your signer…" (Primal is known to silently drop kinds it does not recognise; the pairing never came back). Kinds outside this list are still signed via the
// plain `sign_event` grant (the signer prompts per request), so DON'T extend this without re-testing a fresh Primal pairing.
export const PAIRING_PERM_KINDS = [22242, 27235, 9, 7, 0, 1, 5, 9021, 9022, 9033, 1984, 20001, 20002, 30078, 30030, 30300, 30315, 30623, 30624, 30625];
export const NOSTRCONNECT_PERMS = [
  "get_public_key",
  "sign_event",
  "nip44_encrypt",
  "nip44_decrypt",
  ...PAIRING_PERM_KINDS.map((k) => `sign_event:${k}`),
].join(",");

export type Nip46Session = {
  kind: "nip46";
  clientSecret: string; // hex; ephemeral CLIENT key for the NIP-46 channel only (not the user's identity)
  remotePubkey: string;
  relays: string[];
  userPubkey: string;
};

export class Nip46Error extends Error {
  constructor(
    public code:
      | "timeout"
      | "rejected"
      | "bad-uri"
      | "unreachable"
      | "protocol",
    message: string,
  ) {
    super(message);
    this.name = "Nip46Error";
  }
}

export type BunkerParams = {
  remotePubkey: string;
  relays: string[];
  secret?: string;
};
export function parseBunkerUrl(input: string): BunkerParams {
  let u: URL;
  try {
    u = new URL(input.trim());
  } catch {
    throw new Nip46Error("bad-uri", "That doesn't look like a bunker:// URL.");
  }
  if (u.protocol !== "bunker:")
    throw new Nip46Error("bad-uri", "Expected a URL starting with bunker://");
  const remotePubkey = (
    u.hostname || u.pathname.replace(/^\/+/, "")
  ).toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(remotePubkey))
    throw new Nip46Error(
      "bad-uri",
      "bunker:// URL has no valid signer public key.",
    );
  const relays = u.searchParams
    .getAll("relay")
    .filter((r) => /^wss?:\/\//.test(r));
  if (!relays.length)
    throw new Nip46Error("bad-uri", "bunker:// URL has no relay= parameter.");
  return {
    remotePubkey,
    relays,
    secret: u.searchParams.get("secret") ?? undefined,
  };
}

export function buildNostrConnectUri(
  clientPubkey: string,
  relays: string[],
  secret: string,
): string {
  const p = new URLSearchParams();
  for (const r of relays) p.append("relay", r);
  p.set("secret", secret);
  p.set("name", "Buzz");
  p.set("url", location.origin);
  p.set("perms", NOSTRCONNECT_PERMS);
  return `nostrconnect://${clientPubkey}?${p.toString().replace(/\+/g, "%20")}`;
}

type Pending = {
  id: string; // request id inside the encrypted payload (responses are matched on this)
  event: { id: string; [k: string]: unknown }; // the wire event, republished on reconnect
  resolve: (r: string) => void;
  reject: (e: Error) => void;
  timer: ReturnType<typeof setTimeout>;
  method: string;
  params: string[];
  /** sockets this request was already published on: a request goes to each signer relay ONCE (no multiplied load). */
  sentOn: WeakSet<WebSocket>;
  at: number;
  rechecked?: boolean;
  /** how many times a FRESH wire event (new event id, same request id) was published for this request */
  resends: number;
  lastSentAt: number;
  timeoutMs: number;
};
type RelayConn = {
  url: string;
  ws: WebSocket | null;
  retry: number;
  timer?: ReturnType<typeof setTimeout>;
};

/** Low-level NIP-46 transport: signer relays, reconnect, resubscribe, request/response matching. */
export class Nip46Client {
  readonly clientPubkey: string;
  private clientSk: Uint8Array;
  private conns: RelayConn[] = [];
  private pending = new Map<string, Pending>();
  private seen = new Set<string>();
  private closed = false;
  private useNip04 = false;
  private since = Math.floor(Date.now() / 1000) - 30;
  /** true while a nostrconnect:// handshake is pending: sockets are force-reopened on every foreground. */
  handshake = false;
  /** Used by nostrconnect:// to receive the unsolicited connect response. */
  onUnsolicited?: (ev: { pubkey: string; content: string; id: string }) => void;
  private hiddenAt = 0;
  private watchdog?: ReturnType<typeof setInterval>;
  private onVis = () => {
    if (document.visibilityState === "hidden") {
      this.hiddenAt = Date.now();
      return;
    }
    // Back from a signer app (Primal etc.): iOS may have frozen sockets that still claim OPEN → replace them, resubscribe with `since`.
    const away = this.hiddenAt ? Date.now() - this.hiddenAt : 0;
    this.hiddenAt = 0;
    // Back from the signer app with requests still waiting (e.g. the user just switched Clave from Medium to Full access):
    // the signer already saw (and dropped) our original event id and relays de-duplicate it, so republishing the same event
    // never reaches it again. Re-sign a fresh event for every pending request and give it a full deadline again.
    if (this.pending.size && away > 1500) void this.resendPending("foreground", true);
    this.reconnectAll(away > 3000);
  };
  /** Every 15s while visible: reopen dead signer sockets, and if a request has gone unanswered for 10s, rebuild the sockets once. */
  private health = () => {
    if (this.closed || document.visibilityState !== "visible") return;
    for (const c of this.conns)
      if (!c.timer && (!c.ws || c.ws.readyState === WebSocket.CLOSED)) this.open(c);
    const now = Date.now();
    for (const p of this.pending.values())
      if (!p.rechecked && now - p.at > 10_000) {
        p.rechecked = true;
        nlog(`request ${p.method} unanswered for 10s: rebuilding signer sockets`);
        this.reconnectAll(true);
        break;
      }
    // Handshake requests (connect/get_public_key) that stay unanswered get a fresh event with backoff (20s, 40s, 60s…).
    if ([...this.pending.values()].some((p) => HANDSHAKE_METHODS.has(p.method) && now - p.lastSentAt > Math.min(60_000, 20_000 * 2 ** p.resends)))
      void this.resendPending("backoff", false);
  };

  /** Re-sign and republish every pending request as a NEW event (same request id, so either answer matches). */
  async resendPending(why: string, extendDeadline: boolean) {
    if (this.closed || !this.remotePubkey) return;
    for (const p of [...this.pending.values()]) {
      if (why === "backoff" && !HANDSHAKE_METHODS.has(p.method)) continue;
      try {
        const content = await this.encrypt(this.remotePubkey, JSON.stringify({ id: p.id, method: p.method, params: p.params }));
        p.event = finalizeEvent(
          { kind: NIP46_KIND, created_at: Math.floor(Date.now() / 1000), tags: [["p", this.remotePubkey]], content },
          this.clientSk,
        ) as unknown as Pending["event"];
      } catch {
        continue;
      }
      if (!this.pending.has(p.id)) continue;
      p.resends++;
      p.lastSentAt = Date.now();
      p.sentOn = new WeakSet();
      if (extendDeadline) {
        clearTimeout(p.timer);
        p.timer = setTimeout(() => this.fail(p, "timeout"), p.timeoutMs);
        p.at = Date.now();
        p.rechecked = false;
      }
      nlog(`re-sending ${p.method} as a fresh event (${why}, #${p.resends})`);
      for (const c of this.conns)
        if (c.ws?.readyState === WebSocket.OPEN) {
          p.sentOn.add(c.ws);
          c.ws.send(JSON.stringify(["EVENT", p.event]));
        }
    }
  }

  constructor(
    clientSecretHex: string,
    public remotePubkey: string | null,
    public relays: string[],
    since?: number,
  ) {
    if (since !== undefined) this.since = since;
    this.clientSk = hexToBytes(clientSecretHex);
    this.clientPubkey = getPublicKey(this.clientSk);
  }
  get clientSecret() {
    return bytesToHex(this.clientSk);
  }

  start() {
    this.closed = false;
    this.conns = this.relays.map((url) => ({ url, ws: null, retry: 0 }));
    for (const c of this.conns) this.open(c);
    document.addEventListener("visibilitychange", this.onVis);
    window.addEventListener("online", this.onVis);
    window.addEventListener("pageshow", this.onVis);
    clearInterval(this.watchdog);
    this.watchdog = setInterval(this.health, 15_000);
  }
  stop() {
    this.closed = true;
    clearInterval(this.watchdog);
    document.removeEventListener("visibilitychange", this.onVis);
    window.removeEventListener("online", this.onVis);
    window.removeEventListener("pageshow", this.onVis);
    for (const c of this.conns) {
      clearTimeout(c.timer);
      try {
        c.ws?.close();
      } catch {}
      c.ws = null;
    }
    for (const [id, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(new Nip46Error("unreachable", "Signer connection closed."));
      this.pending.delete(id);
    }
    signerStatus.set({ pending: 0, authUrl: null });
  }

  /** iOS/Android suspend background sockets; on foreground drop dead ones, reopen, resubscribe with `since`. */
  reconnectAll(force = false) {
    if (this.closed) return;
    nlog(`foreground/online: reconnecting signer relays${force ? " (forced)" : ""}`);
    for (const c of this.conns) {
      // A socket iOS froze in the background can still claim OPEN; after being away (force) or during the handshake replace it.
      if (!force && !this.handshake && c.ws && c.ws.readyState === WebSocket.OPEN)
        continue;
      clearTimeout(c.timer);
      try {
        c.ws?.close();
      } catch {}
      c.retry = 0;
      this.open(c);
    }
  }

  private open(c: RelayConn) {
    if (this.closed) return;
    let ws: WebSocket;
    try {
      ws = new WebSocket(c.url);
    } catch {
      return this.scheduleRetry(c);
    }
    c.ws = ws;
    ws.onopen = () => {
      c.retry = 0;
      nip46Debug.setRelay(c.url, true);
      nlog(`relay open, subscribing since=${this.since}`, { relay: c.url });
      ws.send(
        JSON.stringify([
          "REQ",
          "nip46",
          { kinds: [NIP46_KIND], "#p": [this.clientPubkey], since: this.since },
        ]),
      );
      // (re)publish anything still waiting: it may have been sent into a dead socket
      for (const p of this.pending.values())
        if (!p.sentOn.has(ws)) {
          p.sentOn.add(ws);
          ws.send(JSON.stringify(["EVENT", p.event]));
        }
    };
    ws.onmessage = (m) => void this.onMsg(String(m.data));
    ws.onclose = () => {
      if (c.ws === ws) {
        nip46Debug.setRelay(c.url, false);
        this.scheduleRetry(c);
      }
    };
    ws.onerror = () => {};
  }
  private scheduleRetry(c: RelayConn) {
    if (this.closed) return;
    c.retry++;
    c.timer = setTimeout(
      () => this.open(c),
      Math.min(15_000, 500 * 2 ** Math.min(c.retry, 5)),
    );
  }

  private async decrypt(peer: string, content: string): Promise<string> {
    // NIP-04 payloads contain "?iv="; NIP-44 are plain base64. Try the likely one first, then the other.
    const tryNip04 = async () => {
      const t = await nip04.decrypt(this.clientSk, peer, content);
      this.useNip04 = true;
      nlog("decrypt ok (nip04)");
      return t;
    };
    const tryNip44 = () => {
      const t = nip44.v2.decrypt(
        content,
        nip44.v2.utils.getConversationKey(this.clientSk, peer),
      );
      nlog("decrypt ok (nip44)");
      return t;
    };
    const order = content.includes("?iv=")
      ? [tryNip04, tryNip44]
      : [tryNip44, tryNip04];
    let last: unknown;
    for (const f of order) {
      try {
        return await f();
      } catch (e) {
        last = e;
      }
    }
    nlog("decrypt FAILED (nip44 and nip04)");
    throw last;
  }
  private async onMsg(raw: string) {
    let msg: unknown[];
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }
    if (msg[0] !== "EVENT") return;
    const ev = msg[2] as {
      id: string;
      pubkey: string;
      content: string;
      kind: number;
      sig: string;
    };
    if (!ev || ev.kind !== NIP46_KIND || this.seen.has(ev.id)) return;
    this.seen.add(ev.id);
    nlog(`event received kind=${ev.kind} from=${short(ev.pubkey)}`);
    nip46Debug.event(`kind ${ev.kind} from ${short(ev.pubkey)}`);
    if (!verifyEvent(ev as never)) {
      nlog("event dropped: bad signature");
      return;
    }
    if (this.remotePubkey && ev.pubkey !== this.remotePubkey) {
      nlog("event dropped: not from our signer");
      return; // ignore strangers
    }
    let body: { id?: string; result?: string; error?: string; method?: string };
    try {
      body = JSON.parse(await this.decrypt(ev.pubkey, ev.content));
    } catch {
      return;
    }
    const p = body.id ? this.pending.get(body.id) : undefined;
    if (!p) {
      if (!this.remotePubkey && !body.method)
        this.onUnsolicited?.({
          pubkey: ev.pubkey,
          id: ev.id,
          content: JSON.stringify(body),
        });
      return;
    }
    if (body.result === "auth_url" && body.error) {
      // Signer wants the user to approve in a browser/app. Keep waiting, with a longer deadline.
      this.openAuthUrl(body.error);
      clearTimeout(p.timer);
      p.timer = setTimeout(() => this.fail(p, "timeout"), AUTH_URL_TIMEOUT_MS);
      return;
    }
    clearTimeout(p.timer);
    this.pending.delete(body.id as string);
    this.updateStatus();
    // A re-sent bunker connect can be answered "secret already used" because the signer accepted an earlier copy: treat it as
    // connected; finish()'s get_public_key round trip proves we're actually authorized.
    if (body.error && p.method === "connect" && p.resends > 0 && /secret|already/i.test(body.error)) {
      nlog(`connect re-send answered "${body.error}": treating as connected, verifying via get_public_key`);
      signerHealth.success();
      p.resolve("ack");
      return;
    }
    if (body.error) {
      signerHealth.error(body.error);
      p.reject(new Nip46Error("rejected", body.error));
    } else {
      signerHealth.success();
      p.resolve(body.result ?? "");
    }
  }
  private openAuthUrl(url: string) {
    if (!/^https?:\/\//i.test(url)) return;
    signerStatus.set({ authUrl: url });
    try {
      window.open(url, "_blank", "noopener"); // may be blocked on iOS (no user gesture); the status toast has a tap-to-open link
    } catch {}
  }
  private fail(p: Pending, code: "timeout") {
    this.pending.delete(p.id);
    // encrypt/decrypt are background work (settings, key wraps, snapshots); only signatures/handshakes count toward "unresponsive"
    signerHealth.timeout({ background: /crypt/.test(p.method) });
    this.updateStatus();
    p.reject(
      new Nip46Error(
        code,
        `Your signer didn't respond to "${p.method}". Open your signer app and try again.`,
      ),
    );
  }
  private updateStatus() {
    signerStatus.set({
      pending: this.pending.size,
      authUrl: this.pending.size ? signerStatus.get().authUrl : null,
    });
  }

  private async encrypt(peer: string, text: string): Promise<string> {
    return this.useNip04
      ? await nip04.encrypt(this.clientSk, peer, text)
      : nip44.v2.encrypt(
          text,
          nip44.v2.utils.getConversationKey(this.clientSk, peer),
        );
  }

  /** Send one request; resolves with the `result` string. */
  async request(
    method: string,
    params: string[],
    timeoutMs = REQUEST_TIMEOUT_MS,
  ): Promise<string> {
    if (!this.remotePubkey)
      throw new Nip46Error("protocol", "No remote signer pubkey yet.");
    if (this.closed) this.start();
    const id = crypto.randomUUID();
    const content = await this.encrypt(
      this.remotePubkey,
      JSON.stringify({ id, method, params }),
    );
    const event = finalizeEvent(
      {
        kind: NIP46_KIND,
        created_at: Math.floor(Date.now() / 1000),
        tags: [["p", this.remotePubkey]],
        content,
      },
      this.clientSk,
    );
    return new Promise<string>((resolve, reject) => {
      const p: Pending = {
        id,
        event: event as unknown as Pending["event"],
        resolve,
        reject,
        method,
        params,
        sentOn: new WeakSet(),
        at: Date.now(),
        resends: 0,
        lastSentAt: Date.now(),
        timeoutMs,
        timer: setTimeout(() => this.fail(p, "timeout"), timeoutMs),
      };
      this.pending.set(id, p);
      this.updateStatus();
      let sent = false;
      for (const c of this.conns)
        if (c.ws?.readyState === WebSocket.OPEN) {
          if (!p.sentOn.has(c.ws)) {
            p.sentOn.add(c.ws);
            c.ws.send(JSON.stringify(["EVENT", event]));
          }
          sent = true;
        }
      if (!sent) this.reconnectAll(); // onopen will publish pending
    });
  }

  /**
   * nostrconnect://: wait until a signer answers our URI. Accepts result == secret or "ack"; a signer that returns an empty
   * result is accepted tentatively (finish() then proves it via get_public_key). A different non-empty result is ignored.
   */
  waitForConnect(
    secret: string,
    timeoutMs = CONNECT_TIMEOUT_MS,
    signal?: AbortSignal,
  ): Promise<string> {
    return new Promise((resolve, reject) => {
      const done = () => {
        clearTimeout(t);
        this.onUnsolicited = undefined;
      };
      const t = setTimeout(() => {
        done();
        reject(
          new Nip46Error(
            "timeout",
            "No signer answered. Open the link in your signer app and approve the connection, then try again.",
          ),
        );
      }, Math.max(1000, timeoutMs));
      signal?.addEventListener("abort", () => {
        done();
        reject(new Nip46Error("protocol", "Cancelled."));
      });
      this.onUnsolicited = (ev) => {
        try {
          const b = JSON.parse(ev.content) as {
            result?: string;
            error?: string;
          };
          if (b.error) {
            nlog(`connect response: error from ${short(ev.pubkey)}`);
            done();
            return reject(new Nip46Error("rejected", b.error));
          }
          if (b.result === secret || b.result === "ack" || !b.result) {
            nlog(
              `connect response ok from ${short(ev.pubkey)} (${
                b.result === secret
                  ? "secret echoed"
                  : b.result === "ack"
                    ? "ack"
                    : "empty result; will verify via get_public_key"
              })`,
            );
            done();
            resolve(ev.pubkey);
          } else {
            nlog("connect response ignored: result != secret/ack (bad secret)");
          }
        } catch {}
      };
    });
  }
}

let currentClient: Nip46Client | null = null;
/** Client of an in-flight bunker sign-in, so the unlock screen's "Try again" can re-send without a page refresh. */
let handshakeClient: Nip46Client | null = null;
export function nudgeSignerHandshake() {
  const c = handshakeClient;
  if (!c) return false;
  c.reconnectAll(true);
  void c.resendPending("manual", true);
  return true;
}

/**
 * Boot/mid-session liveness probe: a real round trip (get_public_key must come back as the saved pubkey) within `timeoutMs`.
 * On a miss it rebuilds the signer relay sockets and tries once more before reporting failure.
 */
export async function probeNip46(userPubkey: string, timeoutMs = 8000, onAttempt?: (attempt: number, timeoutMs: number) => void): Promise<{ ok: true } | { ok: false; reason: "timeout" | "revoked" | "mismatch" | "no-client" }> {
  const c = currentClient;
  if (!c) return { ok: false, reason: "no-client" };
  signerHealth.reconnecting(true);
  try {
    for (let attempt = 0; attempt < 2; attempt++) {
      if (attempt === 1) c.reconnectAll(true);
      onAttempt?.(attempt, timeoutMs);
      try {
        const pk = await c.request("get_public_key", [], timeoutMs);
        if (pk !== userPubkey) return { ok: false, reason: "mismatch" };
        signerHealth.success();
        return { ok: true };
      } catch (e) {
        if (e instanceof Nip46Error && e.code === "rejected") return { ok: false, reason: "revoked" };
        // timeout or unreachable: retry once after reconnecting sockets
      }
    }
    return { ok: false, reason: "timeout" };
  } finally {
    signerHealth.reconnecting(false);
  }
}

export function createNip46SignerFromClient(
  client: Nip46Client,
  userPubkey: string,
): Signer & { client: Nip46Client } {
  currentClient = client;
  return {
    kind: "nip46",
    client,
    getPublicKey: async () => userPubkey,
    async signEvent(ev: UnsignedEvent): Promise<SignedEvent> {
      const draft = {
        kind: ev.kind,
        content: ev.content,
        tags: ev.tags,
        created_at: ev.created_at ?? Math.floor(Date.now() / 1000),
      };
      const res = await client.request("sign_event", [JSON.stringify(draft)]);
      let signed: SignedEvent;
      try {
        signed = JSON.parse(res);
      } catch {
        throw new Nip46Error("protocol", "Signer returned an invalid event.");
      }
      if (signed.pubkey !== userPubkey || !verifyEvent(signed as never))
        throw new Nip46Error(
          "protocol",
          "Signer returned an event signed by a different key or with a bad signature.",
        );
      if (signed.kind !== draft.kind || signed.content !== draft.content)
        throw new Nip46Error("protocol", "Signer modified the event.");
      return signed;
    },
    nip44: {
      encrypt: (peer, pt) => client.request("nip44_encrypt", [peer, pt]),
      decrypt: (peer, ct) => client.request("nip44_decrypt", [peer, ct]),
    },
  };
}

const newClientKey = () => bytesToHex(generateSecretKey());

async function finish(
  client: Nip46Client,
  relays: string[],
): Promise<Signer & { client: Nip46Client }> {
  nlog("sending get_public_key");
  const userPubkey = await client.request(
    "get_public_key",
    [],
    CONNECT_TIMEOUT_MS,
  );
  nlog(`get_public_key result ${short(userPubkey)}`);
  if (!/^[0-9a-f]{64}$/.test(userPubkey))
    throw new Nip46Error("protocol", "Signer returned an invalid public key.");
  await saveNip46Session({
    kind: "nip46",
    clientSecret: client.clientSecret,
    remotePubkey: client.remotePubkey as string,
    relays,
    userPubkey,
  });
  return createNip46SignerFromClient(client, userPubkey);
}

/** Flow (a): bunker://<remote-pubkey>?relay=..&secret=.. */
export async function connectBunker(
  bunkerUrl: string,
): Promise<Signer & { client: Nip46Client }> {
  const b = parseBunkerUrl(bunkerUrl);
  const client = new Nip46Client(newClientKey(), b.remotePubkey, b.relays);
  client.start();
  handshakeClient = client;
  try {
    const r = await client.request(
      "connect",
      [b.remotePubkey, b.secret ?? ""],
      CONNECT_TIMEOUT_MS,
    );
    if (b.secret && r !== "ack" && r !== b.secret)
      throw new Nip46Error(
        "protocol",
        "Signer answered the connect request with an unexpected secret.",
      );
    return await finish(client, b.relays);
  } catch (e) {
    client.stop();
    throw e;
  } finally {
    if (handshakeClient === client) handshakeClient = null;
  }
}

export type NostrConnectFlow = {
  uri: string;
  connected: Promise<Signer & { client: Nip46Client }>;
  resumed: boolean;
  expiresAt: number;
  /** Abort and forget the pending attempt (user pressed Cancel). */
  cancel: () => void;
  /** Stop listening but KEEP the pending attempt (component unmounted / page going away). */
  detach: () => void;
  client: Nip46Client;
};

function runNostrConnect(p: PendingConnect, resumed: boolean): NostrConnectFlow {
  const client = new Nip46Client(
    p.clientSecret,
    null,
    p.relays,
    p.createdAt - 60,
  );
  client.handshake = true;
  const uri = buildNostrConnectUri(client.clientPubkey, p.relays, p.secret);
  const abort = new AbortController();
  let detached = false;
  nip46Debug.forgetRelays();
  nlog(`${resumed ? "RESUMING" : "starting"} nostrconnect`, {
    relays: p.relays,
    client: short(client.clientPubkey),
    expiresInSec: Math.round((p.expiresAt - Date.now()) / 1000),
  });
  client.start();
  const connected = client
    .waitForConnect(p.secret, p.expiresAt - Date.now(), abort.signal)
    .then(async (remotePubkey) => {
      client.remotePubkey = remotePubkey;
      client.handshake = false;
      const signer = await finish(client, p.relays);
      await clearPendingConnect();
      return signer;
    })
    .catch(async (e) => {
      client.stop();
      if (!detached) {
        await clearPendingConnect();
        nlog(`nostrconnect failed: ${e instanceof Error ? e.message : e}`);
      }
      throw e;
    });
  return {
    uri,
    connected,
    resumed,
    expiresAt: p.expiresAt,
    client,
    cancel: () => abort.abort(),
    detach: () => {
      detached = true;
      abort.abort();
    },
  };
}

/**
 * Flow (b): we mint a nostrconnect:// URI; caller shows it as QR/deep link, then awaits `connected`.
 * The attempt is persisted before the URI is returned, and a still-valid pending attempt is REUSED (never a new keypair)
 * so a reload / tab discard while the user is in the signer app resumes instead of restarting.
 */
export async function beginNostrConnect(
  relays: string[] = DEFAULT_SIGNER_RELAYS,
  opts: { resumeOnly?: boolean } = {},
): Promise<NostrConnectFlow | null> {
  const existing = await loadPendingConnect();
  if (existing) return runNostrConnect(existing, true);
  if (opts.resumeOnly) return null;
  const p: PendingConnect = {
    clientSecret: newClientKey(),
    secret: bytesToHex(generateSecretKey()).slice(0, 32),
    relays,
    createdAt: Math.floor(Date.now() / 1000),
    expiresAt: Date.now() + PENDING_TTL_MS,
  };
  await savePendingConnect(p); // durable BEFORE the link is shown
  return runNostrConnect(p, false);
}
export { clearPendingConnect };

/** Resume from IndexedDB. No network needed up front: userPubkey is stored; sockets open on first use. */
export async function resumeNip46(): Promise<
  (Signer & { client: Nip46Client }) | null
> {
  const s = await loadNip46Session();
  if (!s) return null;
  const client = new Nip46Client(s.clientSecret, s.remotePubkey, s.relays);
  client.start();
  return createNip46SignerFromClient(client, s.userPubkey);
}

export async function forgetNip46(signer?: Signer | null) {
  (signer as { client?: Nip46Client } | null | undefined)?.client?.stop();
  await clearNip46Session();
  await clearPendingConnect();
}

// Back-compat with the slice-1 factory signature.
export function createNip46Signer(_bunkerUri: string): Signer {
  throw new SignerUnavailableError(
    "nip46-not-implemented",
    "Use connectBunker()/startNostrConnect().",
  );
}
