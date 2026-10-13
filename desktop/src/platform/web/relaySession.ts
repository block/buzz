// Relay web session: sign ONE NIP-98 request (POST /auth/session) per session and reuse the returned short-lived
// bearer (`Authorization: BuzzSession <token>`) for HTTP reads (/query) instead of asking the signer per request.
// Read-only and bound server-side to this pubkey + community; expires (8h) and is revoked on sign-out.
// Older relays without /auth/session answer 404/405 → we fall back to per-request NIP-98 and retry later.

type Stored = { base: string; pk: string; token: string; exp: number };
const STORE_KEY = "buzz-relay-session";
/** Refresh this long before the server expiry. */
const EARLY_REFRESH_S = 10 * 60;
/** After an unsupported/failed mint, don't try again for this long. */
const RETRY_AFTER_MS = 10 * 60_000;

let current: Stored | null = null;
let minting: Promise<Stored | null> | null = null;
let backoffUntil = 0;

const now = () => Math.floor(Date.now() / 1000);

function load(base: string, pk: string): Stored | null {
  if (
    current &&
    current.base === base &&
    current.pk === pk &&
    current.exp - EARLY_REFRESH_S > now()
  )
    return current;
  try {
    const s = JSON.parse(
      sessionStorage.getItem(STORE_KEY) || "null",
    ) as Stored | null;
    if (
      s &&
      s.base === base &&
      s.pk === pk &&
      s.exp - EARLY_REFRESH_S > now() &&
      typeof s.token === "string"
    ) {
      current = s;
      return s;
    }
  } catch {
    /* storage unavailable */
  }
  return null;
}

function save(s: Stored | null) {
  current = s;
  try {
    if (s) sessionStorage.setItem(STORE_KEY, JSON.stringify(s));
    else sessionStorage.removeItem(STORE_KEY);
  } catch {
    /* storage unavailable */
  }
}

export type MintDeps = {
  base: string;
  pubkey: string | null;
  /** POST /auth/session with a NIP-98 header; resolves the fetch Response. */
  mint: () => Promise<Response>;
};

/** `BuzzSession <token>` for this relay+pubkey, minting one if needed; null → use per-request NIP-98. */
export async function relaySessionAuth(deps: MintDeps): Promise<string | null> {
  const { base, pubkey } = deps;
  if (!pubkey) return null;
  const hit = load(base, pubkey);
  if (hit) return `BuzzSession ${hit.token}`;
  if (Date.now() < backoffUntil) return null;
  if (!minting) {
    minting = (async () => {
      try {
        const res = await deps.mint();
        if (!res.ok) {
          backoffUntil = Date.now() + RETRY_AFTER_MS;
          return null;
        }
        const j = (await res.json()) as {
          token?: unknown;
          expires_at?: unknown;
        };
        if (typeof j.token !== "string" || typeof j.expires_at !== "number") {
          backoffUntil = Date.now() + RETRY_AFTER_MS;
          return null;
        }
        const s: Stored = {
          base,
          pk: pubkey,
          token: j.token,
          exp: j.expires_at,
        };
        save(s);
        return s;
      } catch {
        backoffUntil = Date.now() + RETRY_AFTER_MS;
        return null;
      } finally {
        minting = null;
      }
    })();
  }
  const s = await minting;
  return s ? `BuzzSession ${s.token}` : null;
}

/** The relay rejected the token (expired/revoked): forget it so the next read mints a fresh one. */
export function dropRelaySession(auth: string) {
  if (current && auth === `BuzzSession ${current.token}`) save(null);
}

/** Revoke server-side (best effort) and forget. Called on sign-out. */
export function revokeRelaySession(
  revoke: (base: string, auth: string) => Promise<unknown>,
) {
  const s =
    current ??
    (() => {
      try {
        return JSON.parse(
          sessionStorage.getItem(STORE_KEY) || "null",
        ) as Stored | null;
      } catch {
        return null;
      }
    })();
  save(null);
  backoffUntil = 0;
  if (s) void revoke(s.base, `BuzzSession ${s.token}`).catch(() => undefined);
}

/** Test hook. */
export function _resetRelaySessionForTests() {
  save(null);
  minting = null;
  backoffUntil = 0;
}
