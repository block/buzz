export interface ReadStateBlob {
  v: 1;
  client_id: string;
  contexts: Record<string, number>;
}

export const READ_STATE_D_TAG_PREFIX = "read-state:";
export const READ_STATE_FETCH_LIMIT = 500;
export const READ_STATE_HORIZON_SECONDS = 7 * 24 * 60 * 60;

export const MAX_CONTEXTS = 10_000;

// Maximum plaintext byte length for the JSON blob passed to nip44EncryptToSelf.
// NIP-44 v2 hard-caps plaintext at 65,535 bytes; the relay enforces a 256 KB
// content limit. 32 KB gives ample headroom for NIP-44 overhead (~1.4×
// expansion to ~45 KB ciphertext) while keeping the blob well under both caps.
export const READ_STATE_MAX_PLAINTEXT_BYTES = 32_768;

// Maximum number of slots a client may publish. Each slot is a separate
// kind:30078 event. Splitting across slots is the fallback when channel keys
// alone exceed READ_STATE_MAX_PLAINTEXT_BYTES. 8 slots × ~650 channel keys per
// slot = ~5,200 channels — well beyond any realistic user.
export const READ_STATE_MAX_SLOTS = 8;

// Context-key prefix for a per-MESSAGE read marker (LP4 v3). One grow-only
// marker per reply id; the badge predicate reads effective("msg:<id>") live so
// reading an ancestor never covers a descendant (Issue 2 by construction).
// Distinct from THREAD_PREFIX so the parent resolver and eviction can tell the
// two key families apart.
export const MSG_PREFIX = "msg:";
export const THREAD_PREFIX = "thread:";

const EVENT_ID_PATTERN = /^[0-9a-f]{64}$/;

export function maxReadAt(...markers: Array<number | null>): number | null {
  return markers.reduce<number | null>((latest, marker) => {
    if (marker === null) return latest;
    if (latest === null || marker > latest) return marker;
    return latest;
  }, null);
}

export function msgContextKey(messageId: string): string {
  return `${MSG_PREFIX}${messageId}`;
}

// Spec-conformance helpers for well-known interoperable context keys. Runtime
// folding/eviction remains prefix-based so opaque client-local keys still work.
export function isThreadContextKey(value: string): value is `thread:${string}` {
  if (!value.startsWith(THREAD_PREFIX)) return false;
  return EVENT_ID_PATTERN.test(value.slice(THREAD_PREFIX.length));
}

export function isMsgContextKey(value: string): value is `msg:${string}` {
  if (!value.startsWith(MSG_PREFIX)) return false;
  return EVENT_ID_PATTERN.test(value.slice(MSG_PREFIX.length));
}

export function localReadStateKey(pubkey: string): string {
  return `buzz.channel-read-state.v2:${pubkey}`;
}

export function localPublishableContextKey(pubkey: string): string {
  return `buzz.channel-read-state.publishable.v1:${pubkey}`;
}

export function localSourceCreatedAtKey(pubkey: string): string {
  return `buzz.channel-read-state.source-created-at.v1:${pubkey}`;
}

export function isPlainRecord(
  value: unknown,
): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isValidBlob(obj: unknown): obj is ReadStateBlob {
  if (!isPlainRecord(obj)) return false;
  const record = obj;
  if (record.v !== 1) return false;
  if (
    typeof record.client_id !== "string" ||
    record.client_id.length === 0 ||
    record.client_id.length > 64
  )
    return false;
  if (!isPlainRecord(record.contexts)) return false;
  if (Object.keys(record.contexts).length > MAX_CONTEXTS) return false;
  return true;
}

export function sanitizeContexts(
  contexts: Record<string, unknown>,
): Record<string, number> {
  const result: Record<string, number> = {};
  for (const [key, value] of Object.entries(contexts)) {
    if (new TextEncoder().encode(key).length > 256) continue;
    if (typeof value !== "number" || !Number.isInteger(value)) continue;
    if (value < 0 || value > 4294967295) continue;
    result[key] = value;
  }
  return result;
}

export function isValidReadStateDTag(
  value: string | undefined,
): value is string {
  if (!value?.startsWith(READ_STATE_D_TAG_PREFIX)) return false;
  const slotId = value.slice(READ_STATE_D_TAG_PREFIX.length);
  return slotId.length > 0 && slotId.length <= 64 && isAscii(slotId);
}

export function localExtraSlotIdsKey(pubkey: string): string {
  return `buzz.nip-rs.extra-slot-ids:${pubkey}`;
}

export function localIsoToUnixSeconds(value: unknown): number | null {
  if (typeof value !== "string" || value.length === 0) {
    return null;
  }

  const ms = Date.parse(value);
  return Number.isNaN(ms) ? null : Math.floor(ms / 1_000);
}

function isAscii(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    if (value.charCodeAt(index) > 0x7f) {
      return false;
    }
  }
  return true;
}
