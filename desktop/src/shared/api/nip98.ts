import { signRelayEvent } from "@/shared/api/tauri";

/** NIP-98 HTTP auth event kind. */
export const NIP98_KIND = 27235;

/** Lowercase hex sha256 of `text`, for the NIP-98 `payload` tag. */
export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * Build the NIP-98 `Authorization` header for a POST with a body.
 *
 * The verifier requires a `payload` tag carrying sha256(body) for signed POSTs
 * and checks the `u` tag against the exact request URL — so the caller must
 * finalize both the URL and the body before signing.
 */
export async function nip98PostHeader(
  url: string,
  body: string,
): Promise<string> {
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
  // NIP-98 events carry empty content and ASCII-only tags, so btoa is safe.
  return `Nostr ${btoa(JSON.stringify(authEvent))}`;
}
