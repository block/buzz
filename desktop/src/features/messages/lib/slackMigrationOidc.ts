import type { ImportClaimDeepLinkPayload } from "@/shared/deep-link";

const VERIFIER_BYTES = 32;

function base64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/** Create the device-held verifier that binds one Slack browser round-trip. */
export function createSlackOidcVerifier(): string {
  return base64Url(crypto.getRandomValues(new Uint8Array(VERIFIER_BYTES)));
}

/** RFC 7636 S256 challenge sent to the migration service before opening Slack. */
export async function slackOidcChallenge(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(verifier),
  );
  return base64Url(new Uint8Array(digest));
}

export async function buildSlackOidcStartUrl(
  service: string,
  verifier: string,
): Promise<string> {
  const base = service.replace(/\/+$/, "");
  const challenge = await slackOidcChallenge(verifier);
  return `${base}/oidc/start?challenge=${encodeURIComponent(challenge)}`;
}

function normalizedUrl(value: string): string | null {
  try {
    const url = new URL(value);
    url.protocol = url.protocol.toLowerCase();
    url.hostname = url.hostname.toLowerCase();
    url.pathname = url.pathname.replace(/\/+$/, "") || "/";
    return url.toString().replace(/\/$/, "");
  } catch {
    return null;
  }
}

/** Ensure an OIDC callback belongs to the join transaction Buzz initiated. */
export function assertMatchesSlackJoin(
  payload: ImportClaimDeepLinkPayload,
  relayUrl: string,
  serviceUrl: string | undefined,
  verifier: string | undefined,
): asserts verifier is string {
  if (
    payload.via !== "oidc" ||
    !payload.relayUrl ||
    !payload.service ||
    !payload.code ||
    !verifier ||
    normalizedUrl(payload.relayUrl) !== normalizedUrl(relayUrl) ||
    normalizedUrl(payload.service) !== normalizedUrl(serviceUrl ?? "")
  ) {
    throw new Error(
      "This Slack response doesn't match the pending community join. Restart from the original Slack join link.",
    );
  }
}
