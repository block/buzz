export interface ReadStateBlob {
  v: 1;
  client_id: string;
  contexts: Record<string, number>;
}

export const READ_STATE_D_TAG_PREFIX = "read-state:";
export const READ_STATE_FETCH_LIMIT = 500;
export const READ_STATE_HORIZON_SECONDS = 7 * 24 * 60 * 60;

export const MAX_CONTEXTS = 10_000;

// Context-key prefix for a per-MESSAGE read marker (LP4 v3). One grow-only
// marker per reply id; the badge predicate reads effective("msg:<id>") live so
// reading an ancestor never covers a descendant (Issue 2 by construction).
// Distinct from THREAD_PREFIX so the parent resolver and eviction can tell the
// two key families apart.
export const MSG_PREFIX = "msg:";
export const THREAD_PREFIX = "thread:";

export function msgContextKey(messageId: string): string {
  return `${MSG_PREFIX}${messageId}`;
}

// A well-formed per-message context key: the msg: prefix with a non-empty id
// that does NOT itself start with thread: (guards against a thread key being
// mistaken for, or double-prefixed into, a message key).
export function isMsgContextKey(value: string): value is `msg:${string}` {
  if (!value.startsWith(MSG_PREFIX)) return false;
  const id = value.slice(MSG_PREFIX.length);
  return id.length > 0 && !id.startsWith(THREAD_PREFIX);
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
