import { stripVTControlCharacters } from 'node:util';

/** Optional bounded NIP-11 relay display name for the footer only. The name is
 * presentation metadata: it never proves connectivity, authorizes anything or
 * replaces the exact configured relay URL. Absence or failure yields no name. */

export const RELAY_NAME_TIMEOUT_MS = 4000;
export const RELAY_NAME_DOCUMENT_BYTES = 32768;
/** Terminal-control characters and bidi overrides are rejected, not re-displayed:
 * newlines, ANSI/CSI sequences, C1 controls, DEL and direction spoofing. */
const TERMINAL_CONTROL = /[\x00-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/;
/** Footer display bound; longer relay names are absurd payloads, not names. */
const RELAY_NAME_DISPLAY_MAX = 64;

/** Sanitized optional `name` from a NIP-11 document. Nothing is invented: absent,
 * non-string, control-bearing, spoofing, empty or absurd names return undefined. */
export function relayNameFromDocument(value: unknown): string | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const name = (value as { name?: unknown }).name;
  if (typeof name !== 'string' || name.length > RELAY_NAME_DISPLAY_MAX) return undefined;
  if (TERMINAL_CONTROL.test(name)) return undefined;
  const sanitized = stripVTControlCharacters(name).trim();
  return sanitized ? sanitized : undefined;
}

/** Relay (ws/wss) URL to its NIP-11 HTTP document URL. wss maps to https; plain ws
 * is accepted only for the loopback fixture origin, mirroring the profile lookup.
 * URLs carrying credentials or fragments never become an HTTP request. */
export function relayNameHttpUrl(relay: string): string | undefined {
  let url: URL;
  try { url = new URL(relay); } catch { return undefined; }
  if (url.username || url.password || url.hash) return undefined;
  if (url.protocol === 'wss:') url.protocol = 'https:';
  else if (url.protocol === 'ws:' && url.hostname === '127.0.0.1') url.protocol = 'http:';
  else return undefined;
  return url.toString();
}

/** One bounded unauthenticated document read. Resolves undefined for every
 * transport, format, size, timeout or cancellation failure — never a rejected
 * promise and never an invented name. A missing name is not a disconnection. */
export async function fetchRelayName(
  relay: string, signal: AbortSignal, fetcher: typeof fetch = fetch,
  timeoutMs = RELAY_NAME_TIMEOUT_MS, maxBytes = RELAY_NAME_DOCUMENT_BYTES,
): Promise<string | undefined> {
  const target = relayNameHttpUrl(relay);
  if (!target || signal.aborted) return undefined;
  const bound = new AbortController();
  const cancel = () => bound.abort();
  signal.addEventListener('abort', cancel, { once: true });
  const timer = setTimeout(cancel, timeoutMs);
  try {
    const response = await fetcher(target, { headers: { Accept: 'application/nostr+json' }, signal: bound.signal, redirect: 'error' });
    if (!response.ok || !response.body) return undefined;
    const reader = response.body.getReader(); let bytes = 0; const chunks: Uint8Array[] = [];
    try {
      while (true) {
        const next = await reader.read();
        if (next.done) break;
        bytes += next.value.length;
        if (bytes > maxBytes) throw Error('Relay name document exceeds limit');
        chunks.push(next.value);
      }
    } finally { await reader.cancel(); reader.releaseLock(); }
    try { return relayNameFromDocument(JSON.parse(Buffer.concat(chunks).toString())); }
    catch { return undefined; }
  } catch { return undefined; } finally { clearTimeout(timer); signal.removeEventListener('abort', cancel); }
}
