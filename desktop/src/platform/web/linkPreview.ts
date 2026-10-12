// Web link preview cards via the Buzz link preview helper (server/link-preview). Only installed when the build sets
// VITE_BUZZ_LINK_PREVIEW_URL; without it the web keeps the desktop-only behaviour (no fetch, plain links).
// Auth: one NIP-98 signature buys a short-lived bearer session from the helper, so previews don't cost a signer round trip each.

export type LinkPreviewMetadata = {
  title: string;
  siteName: string | null;
  description: string | null;
  imageDataUrl: string | null;
  imageDomain: string | null;
  imageFetchState: "none" | "image" | "transient_failure" | "rejected";
  imageRetryAfterMs: number | null;
  faviconDataUrl: string | null;
};

const SESSION_KEY = "buzz-link-preview-session";
let session: { token: string; exp: number } | null = null;
let pendingSession: Promise<string> | null = null;
const inflight = new Map<string, AbortController>();

function readSession() {
  if (session && session.exp * 1000 > Date.now() + 60_000) return session.token;
  try {
    const v = JSON.parse(sessionStorage.getItem(SESSION_KEY) || "null");
    if (v && typeof v.token === "string" && v.exp * 1000 > Date.now() + 60_000) return (session = v).token;
  } catch {
    /* ignore */
  }
  return null;
}

type Sign = (method: string, url: string) => Promise<string>;
const defaultSign: Sign = async (method, url) => (await import("./relayHttp")).nip98Header(method, url, null, 2);

async function token(base: string, fresh: boolean, sign: Sign, fetchImpl: typeof fetch): Promise<string> {
  if (!fresh) {
    const t = readSession();
    if (t) return t;
  }
  pendingSession ??= (async () => {
    const url = `${base}/session`;
    const r = await fetchImpl(url, { method: "POST", headers: { Authorization: await sign("POST", url) } });
    if (!r.ok) throw new Error(`link preview helper: ${r.status}`);
    const v = (await r.json()) as { token: string; exp: number };
    session = v;
    try {
      sessionStorage.setItem(SESSION_KEY, JSON.stringify(v));
    } catch {
      /* ignore */
    }
    return v.token;
  })().finally(() => {
    pendingSession = null;
  });
  return pendingSession;
}

/** Only http(s) links, at most 2 KB. */
export function previewableHref(href: unknown): string | null {
  if (typeof href !== "string" || href.length > 2048) return null;
  try {
    const u = new URL(href);
    return u.protocol === "http:" || u.protocol === "https:" ? u.toString() : null;
  } catch {
    return null;
  }
}

export function makeLinkPreviewHandlers(base: string, fetchImpl: typeof fetch = (...a) => fetch(...a), sign: Sign = defaultSign) {
  base = base.replace(/\/+$/, "");
  const get = async (href: string, signal: AbortSignal, retry = true): Promise<LinkPreviewMetadata | null> => {
    const r = await fetchImpl(`${base}/preview?url=${encodeURIComponent(href)}`, {
      headers: { Authorization: `Bearer ${await token(base, !retry, sign, fetchImpl)}` },
      signal,
    });
    if (r.status === 401 && retry) return get(href, signal, false);
    if (r.status === 204 || r.status === 404) return null;
    if (!r.ok) throw new Error(`link preview helper: ${r.status}`);
    return (await r.json()) as LinkPreviewMetadata | null;
  };
  return {
    fetch_link_preview_metadata: async (a?: { href?: string; requestId?: string }) => {
      const href = previewableHref(a?.href);
      if (!href) return null;
      const ac = new AbortController();
      if (a?.requestId) inflight.set(a.requestId, ac);
      return get(href, ac.signal);
    },
    cancel_link_preview_metadata: (a?: { requestId?: string }) => {
      if (a?.requestId) inflight.get(a.requestId)?.abort();
    },
    release_link_preview_metadata: (a?: { requestId?: string }) => {
      if (a?.requestId) inflight.delete(a.requestId);
    },
  };
}
