// Web Push client for the relay's push gateway (`buzz-web` NIP-PL profile):
//   1. subscribe the browser with the gateway's VAPID key,
//   2. enroll the subscription at the gateway (NIP-98, the member's own signer),
//   3. delegate it to this relay's push executor -> opaque endpoint grant,
//   4. publish a NIP-44-encrypted kind:30350 lease telling the relay which events to push.
// The relay enforces channel access and fans out; the gateway only ever sees content-free hints.
import { nip98Header, submitSignedEvent } from "../relayHttp";
import { getSigner, signerPubkey } from "../signer";
import { relayHttpBase } from "../relayConfig";
import { readPushEnabled, WEB_PUSH_FLAG, WEB_PUSH_URL } from "./config";
import {
  classifyPushEnv,
  pushPrefsFromSettings,
  readPushEnv,
  type PushUiState,
} from "./env";
import {
  KIND_PUSH_LEASE,
  leasePlaintext,
  leaseSubscriptions,
  leaseTags,
  parsePushDescriptor,
  type PushDescriptor,
} from "./lease";

const NOTIFICATION_SETTINGS_KEY = "buzz-notification-settings.v2";
const STATE_KEY = "buzz-web-push-state.v2";
const ACTIVATION_EVENT = "buzz:desktop-notification-action";
const RENEW_BEFORE_SECS = 3 * 24 * 3600;

type Stored = {
  pubkey: string;
  endpoint: string;
  handle: string;
  fingerprint: string;
  leaseExpires: number;
  generation: number;
};

export const pushUiState = (): PushUiState =>
  WEB_PUSH_FLAG ? classifyPushEnv(readPushEnv()) : "unsupported";

const nowSecs = () => Math.floor(Date.now() / 1000);
function load(): Stored | null {
  try {
    return JSON.parse(localStorage.getItem(STATE_KEY) || "null");
  } catch {
    return null;
  }
}
function save(s: Stored | null) {
  try {
    if (s) localStorage.setItem(STATE_KEY, JSON.stringify(s));
    else localStorage.removeItem(STATE_KEY);
  } catch {
    /* storage denied */
  }
}
function prefs() {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(NOTIFICATION_SETTINGS_KEY);
  } catch {
    /* ignore */
  }
  return pushPrefsFromSettings(raw);
}
function b64urlToBytes(s: string): Uint8Array<ArrayBuffer> {
  const pad = "=".repeat((4 - (s.length % 4)) % 4);
  const bin = atob((s + pad).replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

async function gatewayPost<T>(path: string, body: unknown): Promise<T> {
  const url = `${WEB_PUSH_URL}${path}`;
  const bytes = new TextEncoder().encode(JSON.stringify(body));
  const auth = await nip98Header("POST", url, bytes, 2);
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: auth },
    body: bytes,
  });
  if (!res.ok) throw new Error(`push gateway ${path} -> ${res.status}`);
  return (await res.json()) as T;
}

async function relayDescriptor(): Promise<PushDescriptor | null> {
  const res = await fetch(`${relayHttpBase()}/info`, {
    headers: { Accept: "application/nostr+json" },
  }).catch(() => null);
  if (!res?.ok) return null;
  return parsePushDescriptor(await res.json().catch(() => null));
}

async function registration(): Promise<ServiceWorkerRegistration | null> {
  if (!("serviceWorker" in navigator)) return null;
  return (
    (await navigator.serviceWorker.getRegistration()) ??
    (await navigator.serviceWorker.ready)
  );
}

async function publishLease(
  d: PushDescriptor,
  handle: string,
  generation: number,
  grant: string | null,
  me: string,
  expiration: number,
) {
  const signer = getSigner();
  if (!signer.nip44) throw new Error("signer has no NIP-44 support");
  const plaintext = leasePlaintext({
    descriptor: d,
    generation,
    grant,
    subscriptions: leaseSubscriptions(me, prefs()),
  });
  const content = await signer.nip44.encrypt(d.executorPubkey, plaintext);
  const ev = await signer.signEvent(
    { kind: KIND_PUSH_LEASE, content, tags: leaseTags(handle, d, expiration) },
    { priority: 2 },
  );
  await submitSignedEvent(ev as never);
}

/** Subscribe + enroll + delegate + lease. Re-signs only when account, endpoint, prefs or lease age change. */
export async function ensurePushSubscribed(): Promise<boolean> {
  if (!WEB_PUSH_FLAG || !readPushEnabled() || pushUiState() !== "ready")
    return false;
  const me = signerPubkey();
  const reg = await registration();
  if (!me || !reg) return false;
  const d = await relayDescriptor();
  if (!d) return false; // relay doesn't offer buzz-web push: stay quiet
  let sub = await reg.pushManager.getSubscription();
  if (!sub) {
    const res = await fetch(`${WEB_PUSH_URL}/v1/web/vapid-public-key`);
    if (!res.ok) return false;
    const { key } = (await res.json()) as { key: string };
    sub = await reg.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: b64urlToBytes(key),
    });
  }
  const fingerprint = JSON.stringify([
    me,
    sub.endpoint,
    prefs(),
    d.origin,
    d.executorKeyId,
  ]);
  const prev = load();
  if (
    prev &&
    prev.fingerprint === fingerprint &&
    prev.leaseExpires - nowSecs() > RENEW_BEFORE_SECS
  )
    return true;

  const json = sub.toJSON() as {
    endpoint: string;
    keys: { p256dh: string; auth: string };
  };
  const ttl = Math.min(d.maxLeaseTtl, 30 * 24 * 3600);
  const expiration = nowSecs() + ttl;
  let handle =
    prev && prev.pubkey === me && prev.endpoint === sub.endpoint
      ? prev.handle
      : "";
  if (!handle) {
    const r = await gatewayPost<{ installation_handle: string }>(
      "/v1/web/installations",
      {
        v: 1,
        subscription: {
          endpoint: json.endpoint,
          keys: { p256dh: json.keys.p256dh, auth: json.keys.auth },
        },
        expires_at: expiration + 3600,
      },
    );
    handle = r.installation_handle;
  }
  const generation = Math.max(nowSecs(), (prev?.generation ?? 0) + 1);
  const { endpoint_grant } = await gatewayPost<{ endpoint_grant: string }>(
    "/v1/web/delegations",
    {
      v: 1,
      installation_handle: handle,
      generation,
      relay_pubkey: d.executorPubkey,
      not_before: nowSecs() - 60,
      expires_at: expiration,
    },
  );
  await publishLease(d, handle, generation, endpoint_grant, me, expiration);
  save({
    pubkey: me,
    endpoint: sub.endpoint,
    handle,
    fingerprint,
    leaseExpires: expiration,
    generation,
  });
  return true;
}

/** User-gesture path: ask permission, then subscribe. */
export async function enablePushFromTap(): Promise<PushUiState> {
  if (
    typeof Notification !== "undefined" &&
    Notification.permission === "default"
  ) {
    await Notification.requestPermission();
  }
  const state = pushUiState();
  if (state === "ready") await ensurePushSubscribed().catch(() => false);
  return state;
}

/** Turn off: deactivate the lease at the relay, revoke at the gateway, drop the browser subscription. */
export async function disablePush(): Promise<void> {
  const prev = load();
  const me = signerPubkey();
  const d = await relayDescriptor().catch(() => null);
  if (prev && me && d) {
    const generation = Math.max(nowSecs(), prev.generation + 1);
    await publishLease(
      d,
      prev.handle,
      generation,
      null,
      me,
      nowSecs() + 3600,
    ).catch(() => undefined);
    await gatewayPost("/v1/web/installations/revoke", {
      v: 1,
      installation_handle: prev.handle,
    }).catch(() => undefined);
  }
  const reg = await registration();
  await (await reg?.pushManager.getSubscription())
    ?.unsubscribe()
    .catch(() => false);
  save(null);
}

/** Route notification clicks (warm: SW postMessage; cold: ?push= on launch) to the app's activation handler. */
export function installPushClickRouting() {
  const dispatch = (target: unknown) =>
    window.dispatchEvent(new CustomEvent(ACTIVATION_EVENT, { detail: target }));
  navigator.serviceWorker?.addEventListener("message", (e: MessageEvent) => {
    if (e.data?.type === "buzz-push-open" && e.data.target)
      dispatch(e.data.target);
  });
  const params = new URLSearchParams(location.search);
  const raw = params.get("push");
  if (!raw) return;
  params.delete("push");
  const qs = params.toString();
  history.replaceState(
    history.state,
    "",
    `${location.pathname}${qs ? `?${qs}` : ""}${location.hash}`,
  );
  try {
    const target = JSON.parse(raw);
    setTimeout(() => dispatch(target), 2000); // the shell's activation listener mounts after boot
  } catch {
    /* malformed */
  }
}
