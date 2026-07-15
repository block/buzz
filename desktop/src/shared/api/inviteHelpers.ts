export const INVITE_EXPIRED_ERROR = "invite_expired";

/**
 * Parsed invite — either a full (relay + code) or bare-code form.
 *
 * URL inputs (`https://`, `http://`, `buzz://join`) always carry a
 * `relayWsUrl` (already normalised to `ws(s)://`).  A bare code (no scheme,
 * no slashes) omits it — the caller decides which relay to target.
 */
export type ParsedInvite =
  | { relayWsUrl: string; code: string }
  | { code: string };

/**
 * Parse an invite input into a structured form.
 *
 * Accepted input forms:
 *  - `https://<relay>/invite/<code>` → `{ relayWsUrl: "wss://<relay>", code }`
 *  - `http://<relay>/invite/<code>`  → `{ relayWsUrl: "ws://<relay>", code }`
 *  - `buzz://join?relay=<wsUrl>&code=<code>` → `{ relayWsUrl, code }`
 *  - bare code (no `://`, no `/`)    → `{ code }`
 *
 * Returns `null` for empty input or inputs that don't match any form.
 */
export function parseInviteInput(input: string): ParsedInvite | null {
  const trimmed = input.trim();
  if (!trimmed) return null;

  // Try URL-form parse first.
  try {
    const url = new URL(trimmed);

    // buzz://join?relay=...&code=...
    // Non-special schemes put the authority in `host`, not `pathname`.
    if (url.protocol === "buzz:") {
      if (url.host !== "join") return null;
      const relay = url.searchParams.get("relay");
      const code = url.searchParams.get("code");
      if (!relay || !code) return null;
      if (!relay.startsWith("ws://") && !relay.startsWith("wss://"))
        return null;
      if (url.username || url.password || url.hash) return null;
      // Reject credentials or fragments smuggled inside the nested relay param.
      try {
        const relayUrl = new URL(relay);
        if (relayUrl.username || relayUrl.password || relayUrl.hash)
          return null;
      } catch {
        return null;
      }
      return { relayWsUrl: relay, code };
    }

    // https(s)://<relay>/invite/<code>
    if (url.protocol === "https:" || url.protocol === "http:") {
      if (url.username || url.password || url.hash) return null;
      // pathname must be /invite/<code> with optional single trailing slash
      const match = url.pathname.match(/^\/invite\/([^/]+)\/?$/);
      if (!match?.[1]) return null;
      const code = decodeURIComponent(match[1]);
      // Convert scheme: https → wss, http → ws. url.host already includes port.
      const relayWsUrl =
        url.protocol === "https:" ? `wss://${url.host}` : `ws://${url.host}`;
      return { relayWsUrl, code };
    }

    // ws/wss or any other scheme — not an invite URL.
    return null;
  } catch {
    // Not a URL — fall through to bare-code check.
  }

  // Bare code: no scheme, no slashes.
  if (trimmed.includes("://") || trimmed.includes("/")) return null;
  return { code: trimmed };
}

/** Convert a ws(s) relay URL to its http(s) equivalent. */
export function relayHttpFromWs(wsUrl: string): string {
  if (wsUrl.startsWith("wss://")) return `https://${wsUrl.slice(6)}`;
  if (wsUrl.startsWith("ws://")) return `http://${wsUrl.slice(5)}`;
  throw new Error(`Expected ws:// or wss:// relay URL, got: ${wsUrl}`);
}

export function inviteErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : `${error}`;
}

export function isInviteExpiredError(error: unknown): boolean {
  return inviteErrorMessage(error) === INVITE_EXPIRED_ERROR;
}
