/**
 * NIP-98 HTTP Auth helper — signs a kind:27235 event for authenticating
 * HTTP requests to the relay (used by isomorphic-git for smart HTTP transport).
 */

import { finalizeEvent } from "nostr-tools/pure";
import { getEphemeralKey } from "./nostr-client";

/**
 * Build a NIP-98 Authorization header value.
 *
 * Creates a kind:27235 event with `u` and `method` tags, signs it with the
 * session's ephemeral key, base64-encodes the JSON, and returns
 * `"Nostr <base64>"`.
 */
export function makeNip98AuthHeader(url: string, method: string): string {
  const event = finalizeEvent(
    {
      kind: 27235,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["u", url],
        ["method", method],
      ],
      content: "",
    },
    getEphemeralKey(),
  );

  const json = JSON.stringify(event);
  const base64 = btoa(json);
  return `Nostr ${base64}`;
}
