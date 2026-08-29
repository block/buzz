import { relayHttpFromWs } from "@/shared/api/inviteHelpers";
import { nip98PostHeader } from "@/shared/api/nip98";
import { getRelayHttpUrl, invokeTauri } from "@/shared/api/tauri";

// Relay invite data layer. Both endpoints are NIP-98-authed HTTP POSTs
// (mirrors the read path in moderation.ts, plus the payload tag the relay
// requires for signed POST bodies):
//
// - POST /api/invites        — mint a code (relay checks owner/admin role)
// - POST /api/invites/claim  — claim a code, signed by the *joining* key.
//   This one targets an arbitrary relay (the invite's relay, not necessarily
//   the active community), so the claim helper takes an explicit ws URL.

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
