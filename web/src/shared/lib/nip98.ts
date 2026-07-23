/**
 * NIP-98 HTTP Auth helper — signs a kind:27235 event for authenticating
 * HTTP requests to the relay (used by isomorphic-git for smart HTTP transport).
 */

import { signNostrEvent } from "./nostr-signer";

async function sha256Hex(value: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(value),
  );
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * Build a NIP-98 Authorization header value.
 *
 * Signed POST bodies include the payload digest required by invite endpoints.
 */
export async function makeNip98AuthHeader(
  url: string,
  method: string,
  options?: { body?: string; requireNip07?: boolean },
): Promise<string> {
  const tags = [
    ["u", url],
    ["method", method],
  ];
  if (options?.body !== undefined) {
    tags.push(["payload", await sha256Hex(options.body)]);
    tags.push(["nonce", crypto.randomUUID()]);
  }
  const event = await signNostrEvent(
    {
      kind: 27235,
      tags,
      content: "",
    },
    { requireNip07: options?.requireNip07 },
  );

  const json = JSON.stringify(event);
  const base64 = btoa(json);
  return `Nostr ${base64}`;
}
