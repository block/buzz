import { relayHttpFromWs } from "@/shared/api/inviteHelpers";
import {
  getRelayHttpUrl,
  invokeTauri,
  signRelayEvent,
} from "@/shared/api/tauri";

// Relay invite data layer. Both endpoints are NIP-98-authed HTTP POSTs
// (mirrors the read path in moderation.ts, plus the payload tag the relay
// requires for signed POST bodies):
//
// - POST /api/invites        — mint a code (relay checks owner/admin role)
// - POST /api/invites/claim  — claim a code, signed by the *joining* key.
//   This one targets an arbitrary relay (the invite's relay, not necessarily
//   the active community), so the claim helper takes an explicit ws URL.
//
// Canonical-origin signing law: the NIP-98 `u` tag is signed against the
// origin the relay ADVERTISES in GET /info, while the HTTP request rides
// the transport host the caller supplied — sign the identity, ride the
// road (an alias host like relay2.skaists.dev serving the canonical
// beehivenature.buzz identity otherwise fails invite auth).

const NIP98_KIND = 27235;

// Bound invite requests so an unreachable relay surfaces as an error in the
// invite-loading UI within seconds instead of hanging for the OS-level
// connect timeout (a minute or more on macOS).
const INVITE_REQUEST_TIMEOUT_MS = 15_000;

export type MintedInvite = {
  code: string;
  expiresAt: number;
  url: string;
  maxUses: number | null;
  usesRemaining: number | null;
};

export type JoinPolicy = {
  termsMarkdown?: string;
  privacyMarkdown?: string;
  ageAttestationRequired: boolean;
  version: string;
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

/**
 * The canonical HTTP origin this relay advertises for itself, from GET
 * `/info` on the TRANSPORT road.
 *
 * A deployment's identity can differ from the host a client rides
 * (`relay2.skaists.dev` advertises `wss://beehivenature.buzz`): the relay
 * verifies NIP-98 `u` tags against the canonical origin, so clients must
 * SIGN the identity while RIDING the road.
 *
 * Fail-closed rules (review round 1):
 * - An UNREADABLE /info document (invalid JSON, or a decoded non-object
 *   root such as `null`/array) stops the request before signing or POST —
 *   it proves nothing about what the relay advertises, and silently
 *   signing the transport host instead is the alias-host auth bug itself.
 *   Only a well-formed info OBJECT with no advertised origin means "the
 *   road IS the identity" (plain relays keep working).
 * - The advertisement must be a STRUCTURAL ws/wss ORIGIN — no userinfo,
 *   query, fragment, or non-root path; port and IPv6 literals are valid.
 *   Components beyond the origin would land inside the signed target.
 * - The HTTP signing origin is CONSTRUCTED from the parsed/validated URL
 *   (never by concatenating the raw advertisement string).
 * - No DNS/network resolution is performed as validation.
 */
async function canonicalSigningBase(
  transportHttpBase: string,
): Promise<string> {
  const infoUrl = `${transportHttpBase.replace(/\/+$/, "")}/info`;
  const response = await fetch(infoUrl, {
    signal: AbortSignal.timeout(INVITE_REQUEST_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new Error(`relay /info HTTP ${response.status}`);
  }
  const text = await response.text();
  let decoded: unknown;
  try {
    decoded = JSON.parse(text);
  } catch {
    throw new Error("relay /info returned a malformed document");
  }
  if (
    typeof decoded !== "object" ||
    decoded === null ||
    Array.isArray(decoded)
  ) {
    throw new Error("relay /info returned a malformed document");
  }
  const info = decoded as { push?: unknown };
  let advertised: unknown;
  if (info.push !== undefined) {
    if (
      typeof info.push !== "object" ||
      info.push === null ||
      Array.isArray(info.push)
    ) {
      throw new Error("relay /info returned a malformed push descriptor");
    }
    advertised = (info.push as { origin?: unknown }).origin;
  }
  if (advertised === undefined) {
    return transportHttpBase;
  }
  if (typeof advertised !== "string") {
    throw new Error("relay /info advertises a malformed canonical origin");
  }
  let parsed: URL;
  try {
    parsed = new URL(advertised);
  } catch {
    throw new Error("relay /info canonical origin failed URL verification");
  }
  if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") {
    throw new Error("relay /info canonical origin failed URL verification");
  }
  if (
    parsed.username !== "" ||
    parsed.password !== "" ||
    parsed.search !== "" ||
    parsed.hash !== ""
  ) {
    throw new Error("relay /info canonical origin failed URL verification");
  }
  if (parsed.pathname !== "" && parsed.pathname !== "/") {
    throw new Error("relay /info canonical origin failed URL verification");
  }
  if (parsed.hostname === "") {
    throw new Error("relay /info canonical origin failed URL verification");
  }
  // Constructed from the PARSED url: scheme upgraded ws→http / wss→https,
  // url.host keeps any port and IPv6 bracket literals.
  const scheme = parsed.protocol === "wss:" ? "https" : "http";
  return `${scheme}://${parsed.host}`;
}

async function invitePost<T>(
  httpBase: string,
  path: string,
  body: string,
): Promise<T> {
  const transportUrl = `${httpBase.replace(/\/+$/, "")}${path}`;
  // Sign the canonical identity; request the transport road. With no
  // advertised origin these are the same URL and behavior is unchanged.
  const signingBase = await canonicalSigningBase(httpBase);
  const signedUrl = `${signingBase.replace(/\/+$/, "")}${path}`;
  const authorization = await nip98PostHeader(signedUrl, body);
  const response = await fetch(transportUrl, {
    method: "POST",
    headers: {
      Authorization: authorization,
      "Content-Type": "application/json",
    },
    body,
    signal: AbortSignal.timeout(INVITE_REQUEST_TIMEOUT_MS),
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

/** Absolute URL of a relay-hosted policy document page (system-browser target). */
export function joinPolicyDocumentUrl(
  relayWsUrl: string,
  document: "terms" | "privacy",
): string {
  const base = relayHttpFromWs(relayWsUrl);
  return `${base.replace(/\/+$/, "")}/api/join-policy/${document}`;
}

/** Whether a normalized relay URL is complete enough for background policy discovery. */
export function isJoinPolicyDiscoveryCandidate(relayWsUrl: string): boolean {
  try {
    const url = new URL(relayWsUrl);
    if (url.protocol !== "ws:" && url.protocol !== "wss:") return false;
    return (
      url.hostname === "localhost" ||
      url.hostname === "127.0.0.1" ||
      url.hostname.includes(".")
    );
  } catch {
    return false;
  }
}

/** Fetch relay-hosted policy content for any join surface. */
export async function getJoinPolicy(
  relayWsUrl: string,
  transport: "native" | "webview",
): Promise<JoinPolicy | null> {
  type RawJoinPolicy = {
    terms_markdown?: string;
    privacy_markdown?: string;
    age_attestation_required: boolean;
    version: string;
  };
  let raw: RawJoinPolicy | null;
  if (transport === "native") {
    raw = await invokeTauri<RawJoinPolicy | null>("fetch_join_policy", {
      relayUrl: relayWsUrl,
    });
  } else {
    const base = relayHttpFromWs(relayWsUrl);
    const response = await fetch(`${base.replace(/\/+$/, "")}/api/join-policy`);
    // Relays predating join-policy support have no configured policy.
    if (response.status === 404) return null;
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    raw =
      (
        (await response.json()) as {
          policy?: RawJoinPolicy;
        }
      ).policy ?? null;
  }
  return raw
    ? {
        termsMarkdown: raw.terms_markdown,
        privacyMarkdown: raw.privacy_markdown,
        ageAttestationRequired: raw.age_attestation_required,
        version: raw.version,
      }
    : null;
}

/** Accept the current join policy for an invite and receive a bound receipt. */
export async function acceptJoinPolicy(
  relayWsUrl: string,
  code: string,
  policyVersion: string,
  ageConfirmed: boolean,
): Promise<string> {
  const base = relayHttpFromWs(relayWsUrl);
  const response = await fetch(
    `${base.replace(/\/+$/, "")}/api/invites/accept-policy`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        code,
        policy_version: policyVersion,
        age_confirmed: ageConfirmed,
      }),
    },
  );
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return ((await response.json()) as { receipt: string }).receipt;
}

/** Mint an invite code on the active community's relay (owner/admin only). */
export async function mintInvite(options?: {
  ttlSecs?: number;
  maxUses?: number | null;
}): Promise<MintedInvite> {
  const base = await getRelayHttpUrl();
  const payload: Record<string, unknown> = {};
  if (options?.ttlSecs != null) payload.ttl_secs = options.ttlSecs;
  if (options?.maxUses != null) payload.max_uses = options.maxUses;
  const body = JSON.stringify(payload);
  const raw = await invitePost<{
    code: string;
    expires_at: number;
    url: string;
    max_uses: number | null;
    uses_remaining: number | null;
  }>(base, "/api/invites", body);
  return {
    code: raw.code,
    expiresAt: raw.expires_at,
    url: raw.url,
    maxUses: raw.max_uses,
    usesRemaining: raw.uses_remaining,
  };
}

/**
 * Claim an invite code against `relayWsUrl` (the invite's relay — not
 * necessarily the active community), signed by this app's identity key.
 */
export async function claimInvite(
  relayWsUrl: string,
  code: string,
  policyReceipt?: string,
): Promise<ClaimResult> {
  const base = relayHttpFromWs(relayWsUrl);
  const body = JSON.stringify({ code, policy_receipt: policyReceipt });
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
