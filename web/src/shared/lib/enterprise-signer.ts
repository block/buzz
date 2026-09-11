import type { SignedNostrEvent, UnsignedNostrEvent } from "./nostr-signer";

export type EnterpriseSignerSession = {
  pubkeyHex: string;
  relayWsUrl?: string;
  relayHttpUrl?: string;
  communityId?: string;
  membershipState?: "active" | "pending";
  retryAfterMs?: number;
};

type SignPurpose = "nip42-auth" | "publish" | "http-auth" | "media-upload";

const DEFAULT_TIMEOUT_MS = 10_000;
const HEX_32 = /^[0-9a-f]{64}$/;

export function enterpriseSignerBaseUrl(): string | null {
  const configured = import.meta.env.VITE_ENTERPRISE_SIGNER_BASE_URL?.trim();
  if (!configured) return null;
  return configured.replace(/\/+$/, "");
}

export function isEnterpriseSignerEnabled(): boolean {
  return enterpriseSignerBaseUrl() != null;
}

export class EnterpriseSignerError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EnterpriseSignerError";
  }
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const base = enterpriseSignerBaseUrl();
  if (!base)
    throw new EnterpriseSignerError("Enterprise signer is not configured.");
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), DEFAULT_TIMEOUT_MS);
  try {
    const response = await fetch(`${base}${path}`, {
      method: "POST",
      credentials: "include",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    if (!response.ok) {
      throw new EnterpriseSignerError(
        `Enterprise signer request failed (${response.status}).`,
      );
    }
    return (await response.json()) as T;
  } finally {
    clearTimeout(timeout);
  }
}

function assertUnsignedOnly(event: UnsignedNostrEvent): void {
  const maybeSigned = event as UnsignedNostrEvent & Partial<SignedNostrEvent>;
  if (maybeSigned.pubkey || maybeSigned.id || maybeSigned.sig) {
    throw new EnterpriseSignerError(
      "Enterprise signer templates must not include pubkey, id, or sig.",
    );
  }
}

export async function getEnterpriseSignerSession(): Promise<EnterpriseSignerSession> {
  const session = await postJson<EnterpriseSignerSession>(
    "/v1/buzz/enterprise-signer/session",
    {},
  );
  if (!HEX_32.test(session.pubkeyHex)) {
    throw new EnterpriseSignerError(
      "Enterprise signer returned an invalid pubkey.",
    );
  }
  return session;
}

export async function signWithEnterpriseSigner(
  event: UnsignedNostrEvent,
  purpose: SignPurpose,
): Promise<SignedNostrEvent> {
  assertUnsignedOnly(event);
  const session = await getEnterpriseSignerSession();
  if (session.membershipState === "pending") {
    throw new EnterpriseSignerError(
      "Enterprise relay membership is still provisioning.",
    );
  }
  const response = await postJson<{ event: SignedNostrEvent }>(
    "/v1/buzz/enterprise-signer/events/sign",
    {
      event,
      purpose,
    },
  );
  const signed = response.event;
  if (signed.pubkey !== session.pubkeyHex) {
    throw new EnterpriseSignerError(
      "Enterprise signer returned an event for the wrong account.",
    );
  }
  if (
    signed.kind !== event.kind ||
    signed.created_at !== event.created_at ||
    signed.content !== event.content ||
    JSON.stringify(signed.tags) !== JSON.stringify(event.tags) ||
    !signed.id ||
    !signed.sig
  ) {
    throw new EnterpriseSignerError(
      "Enterprise signer returned an invalid signed event.",
    );
  }
  return signed;
}
