// Web Push client: VAPID subscribe, NIP-98-signed registration with the push sender, preference sync,
// and notification-click routing into the app (reuses the desktop notification activation event).
import { nip98Header } from "../relayHttp";
import { signerPubkey } from "../signer";
import { defaultRelayWsUrl } from "../relayConfig";
import { readPushEnabled, WEB_PUSH_FLAG, WEB_PUSH_URL } from "./config";
import {
  classifyPushEnv,
  pushPrefsFromSettings,
  readPushEnv,
  type PushPrefs,
  type PushUiState,
} from "./env";

const NOTIFICATION_SETTINGS_KEY = "buzz-notification-settings.v2";
const SYNCED_KEY = "buzz-web-push-synced.v1";
const ACTIVATION_EVENT = "buzz:desktop-notification-action";

export const pushUiState = (): PushUiState =>
  WEB_PUSH_FLAG ? classifyPushEnv(readPushEnv()) : "unsupported";

function currentPrefs(): PushPrefs {
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

async function signedFetch(
  method: "PUT" | "DELETE",
  body: unknown,
): Promise<Response> {
  const url = `${WEB_PUSH_URL}/subscriptions`;
  const bytes = new TextEncoder().encode(JSON.stringify(body));
  // Low priority: never block chat traffic on the (slow) remote signer.
  const auth = await nip98Header(method, url, bytes, 2);
  return fetch(url, {
    method,
    headers: { "content-type": "application/json", authorization: auth },
    body: bytes,
  });
}

async function registration(): Promise<ServiceWorkerRegistration | null> {
  if (!("serviceWorker" in navigator)) return null;
  return (
    (await navigator.serviceWorker.getRegistration()) ??
    (await navigator.serviceWorker.ready)
  );
}

/** Subscribe (if needed) and register with the push sender. Only re-signs when endpoint/prefs/account change. */
export async function ensurePushSubscribed(): Promise<boolean> {
  if (!WEB_PUSH_FLAG || !readPushEnabled() || pushUiState() !== "ready")
    return false;
  const pubkey = signerPubkey();
  const reg = await registration();
  if (!pubkey || !reg) return false;
  let sub = await reg.pushManager.getSubscription();
  if (!sub) {
    const res = await fetch(`${WEB_PUSH_URL}/vapid-public-key`);
    if (!res.ok) return false;
    const { key } = (await res.json()) as { key: string };
    sub = await reg.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: b64urlToBytes(key),
    });
  }
  const prefs = currentPrefs();
  const relay = defaultRelayWsUrl();
  const fingerprint = JSON.stringify([pubkey, sub.endpoint, prefs, relay]);
  try {
    if (localStorage.getItem(SYNCED_KEY) === fingerprint) return true;
  } catch {
    /* ignore */
  }
  const res = await signedFetch("PUT", {
    subscription: sub.toJSON(),
    prefs,
    relay,
  });
  if (!res.ok) return false;
  try {
    localStorage.setItem(SYNCED_KEY, fingerprint);
  } catch {
    /* ignore */
  }
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

export async function disablePush(): Promise<void> {
  const reg = await registration();
  const sub = await reg?.pushManager.getSubscription();
  if (!sub) return;
  await signedFetch("DELETE", { endpoint: sub.endpoint }).catch(
    () => undefined,
  );
  await sub.unsubscribe().catch(() => false);
  try {
    localStorage.removeItem(SYNCED_KEY);
  } catch {
    /* ignore */
  }
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
    // The shell's activation listener mounts after boot; dispatch once it is up.
    setTimeout(() => dispatch(target), 2000);
  } catch {
    /* malformed */
  }
}
