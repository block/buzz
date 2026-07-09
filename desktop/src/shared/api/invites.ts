import { relayHttpFromWs } from "@/shared/api/inviteHelpers";
import { getRelayHttpUrl, signRelayEvent } from "@/shared/api/tauri";

// Relay invite data layer. Both endpoints are NIP-98-authed HTTP POSTs
// (mirrors the read path in moderation.ts, plus the payload tag the relay
// requires for signed POST bodies):
//
// - POST /api/invites        — mint a code (relay checks owner/admin role)
// - POST /api/invites/claim  — claim a code, signed by the *joining* key.
//   This one targets an arbitrary relay (the invite's relay, not necessarily
//   the active workspace), so the claim helper takes an explicit ws URL.

const NIP98_KIND = 27235;

export type MintedInvite = {
  code: string;
  expiresAt: number;
  url: string;
};

export type ClaimResult = {
  status: "joined" | "already_member";
  communityId: string;
  host: string;
  role: string;
};

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * Build the NIP-98 `Authorization` header for a POST with a body.
 *
 * The relay requires a `payload` tag carrying sha256(body) for signed POSTs
 * (api/invites.rs passes `require_payload: true`), and verifies the `u` tag
 * against the exact request URL — so the caller finalizes both before signing.
 */
async function nip98PostHeader(url: string, body: string): Promise<string> {
  const authEvent = await signRelayEvent({
    kind: NIP98_KIND,
    content: "",
    tags: [
      ["u", url],
      ["method", "POST"],
      ["payload", await sha256Hex(body)],
      ["nonce", crypto.randomUUID()],
    ],
  });
  // NIP-98 events carry empty content and ASCII-only tags, so btoa is safe here.
  return `Nostr ${btoa(JSON.stringify(authEvent))}`;
}

async function invitePost<T>(
  httpBase: string,
  path: string,
  body: string,
): Promise<T> {
  const url = `${httpBase.replace(/\/+$/, "")}${path}`;
  const authorization = await nip98PostHeader(url, body);
  const response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: authorization,
      "Content-Type": "application/json",
    },
    body,
  });
  const json = (await response.json().catch(() => ({}))) as Record<
    string,
    unknown
  >;
  if (!response.ok) {
    const message =
      typeof json.error === "string" ? json.error : `HTTP ${response.status}`;
    throw new Error(message);
  }
  return json as T;
}

/** Mint an invite code on the active workspace's relay (owner/admin only). */
export async function mintInvite(ttlSecs?: number): Promise<MintedInvite> {
  const base = await getRelayHttpUrl();
  const body = JSON.stringify(ttlSecs ? { ttl_secs: ttlSecs } : {});
  const raw = await invitePost<{
    code: string;
    expires_at: number;
    url: string;
  }>(base, "/api/invites", body);
  return { code: raw.code, expiresAt: raw.expires_at, url: raw.url };
}

/**
 * Claim an invite code against `relayWsUrl` (the invite's relay — not
 * necessarily the active workspace), signed by this app's identity key.
 */
export async function claimInvite(
  relayWsUrl: string,
  code: string,
): Promise<ClaimResult> {
  const base = relayHttpFromWs(relayWsUrl);
  const body = JSON.stringify({ code });
  const raw = await invitePost<{
    status: "joined" | "already_member";
    community_id: string;
    host: string;
    role: string;
  }>(base, "/api/invites/claim", body);
  return {
    status: raw.status,
    communityId: raw.community_id,
    host: raw.host,
    role: raw.role,
  };
}
