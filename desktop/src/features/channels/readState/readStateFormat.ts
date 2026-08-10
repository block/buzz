export interface ReadStateBlob {
  v: 1;
  client_id: string;
  contexts: Record<string, number>;
}

export const READ_STATE_D_TAG_PREFIX = "read-state:";
export const READ_STATE_FETCH_LIMIT = 500;
export const READ_STATE_HORIZON_SECONDS = 7 * 24 * 60 * 60;

export const MAX_CONTEXTS = 10_000;

// Local-storage cap on within-horizon msg:/thread: markers. Generous multiple
// of what the 32 KB publish budget can round-trip (~290 entries), so anything
// beyond it is local-only dead weight that other devices never see anyway.
export const LOCAL_MAX_PRUNABLE_CONTEXTS = 1_000;

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

// ---------------------------------------------------------------------------
// Reserved namespace — override key prefixes and escape marker
//
// NIP-RS spec (Reserved Namespace section): the `ov_` stem and `esc:` marker
// are reserved for the manual-unread override layer.  Any raw context ID that
// starts with either prefix MUST be escaped before use as a frontier key.
// Override counter keys (`ov_s:`, `ov_c:`, `ov_b:`) are NEVER escaped — they
// are their own wire namespace and are not legitimate frontier keys.
// ---------------------------------------------------------------------------

export const OV_S_PREFIX = "ov_s:" as const;
export const OV_C_PREFIX = "ov_c:" as const;
export const OV_B_PREFIX = "ov_b:" as const;
const OV_STEM = "ov_" as const;
const ESC_PREFIX = "esc:" as const;

/** True for wire keys that are override counters (`ov_s:`, `ov_c:`, `ov_b:`). */
export function isOverrideKey(key: string): boolean {
  return (
    key.startsWith(OV_S_PREFIX) ||
    key.startsWith(OV_C_PREFIX) ||
    key.startsWith(OV_B_PREFIX)
  );
}

/**
 * Escape a raw context ID for use as a frontier key.
 * Prepends `esc:` when the raw ID begins with `ov_` or `esc:`.
 * No-op for the context shapes Buzz generates (UUID, msg:hex64, thread:hex64).
 */
export function escapeFrontierKey(rawCtx: string): string {
  if (rawCtx.startsWith(OV_STEM) || rawCtx.startsWith(ESC_PREFIX)) {
    return ESC_PREFIX + rawCtx;
  }
  return rawCtx;
}

/**
 * Recover the raw context ID from a frontier wire key.
 * Strips exactly one leading `esc:` if present; never strips more.
 */
export function unescapeFrontierKey(wireKey: string): string {
  if (wireKey.startsWith(ESC_PREFIX)) {
    return wireKey.slice(ESC_PREFIX.length);
  }
  return wireKey;
}

// ---------------------------------------------------------------------------
// Override register — the (S, C, B) triple for a single context
// ---------------------------------------------------------------------------

/**
 * A fully-validated override register for one context.
 *
 * A live override (`isOverrideActive === true`) carries all three counters.
 * A tombstone floor carries only `c` (S=0, B=0).
 * A virgin register (never activated) is represented by the absence of this
 * type entirely — it is omitted from the wire.
 *
 * Slice 2 (manager) and slice 3 (UI) use this type as the wire-in/wire-out
 * contract.  Slices MUST NOT construct registers directly — use
 * `encodeOverrideGroup` for encoding and read `ParsedReadStateEvent.contexts.overrides`
 * (via `parseContexts`) for decoding.
 */
export interface OverrideRegister {
  /** Set counter S — incremented on each mark-unread. */
  readonly s: number;
  /** Clear counter C — incremented on each mark-read. */
  readonly c: number;
  /** Baseline B — effective frontier at the time of the most recent mark-unread. */
  readonly b: number;
}

/**
 * Result of evaluating whether a context has an active manual-unread override.
 *
 * `active` means: `S > 0 AND F <= B AND S > C` (clear-wins tie policy).
 * `frontier` is the merged effective frontier for the context (used by the
 * unread verdict in the UI layer).
 */
export interface OverrideLiveness {
  readonly active: boolean;
  /** The merged effective frontier value passed to `isOverrideActive`. */
  readonly frontier: number;
}

/**
 * Evaluate the liveness predicate from the spec (clear-wins, mandatory).
 *
 * override_active(S, C, B, F) = S > 0 AND F <= B AND S > C
 *
 * F is the merged effective frontier for the context — typically
 * `contexts.get(rawCtxId) ?? 0` after applying the hierarchical frontier rule.
 */
export function isOverrideActive(
  reg: OverrideRegister,
  frontier: number,
): boolean {
  return reg.s > 0 && frontier <= reg.b && reg.s > reg.c;
}

/**
 * Componentwise `max()` merge of two override registers.
 * Commutative, associative, idempotent — same as the frontier max() merge.
 */
export function mergeOverrideRegisters(
  a: OverrideRegister,
  b: OverrideRegister,
): OverrideRegister {
  return {
    s: Math.max(a.s, b.s),
    c: Math.max(a.c, b.c),
    b: Math.max(a.b, b.b),
  };
}

/**
 * Canonical wire encoding of an override register for a given context.
 *
 * Applies the tombstone-floor / virgin-omit rules from the spec
 * (Mandatory Canonical Publication section) so publishers always emit
 * a correctly-shaped group.
 *
 * Returns a partial `contexts` patch to be merged into the blob before
 * serialization.  Caller is responsible for including the frontier entry
 * for `rawCtxId` separately.
 *
 * @param rawCtxId  Raw (unescaped) context ID — e.g. `"abc-channel-uuid"`.
 * @param reg       Override register to encode.
 * @param frontier  Current merged effective frontier for this context.
 */
export function encodeOverrideGroup(
  rawCtxId: string,
  reg: OverrideRegister,
  frontier: number,
): Record<string, number> {
  const virgin = reg.s === 0 && reg.c === 0;
  if (virgin) {
    // Virgin registers are omitted entirely from the wire.
    return {};
  }
  const active = isOverrideActive(reg, frontier);
  if (active) {
    // Live override: publish all three keys.
    return {
      [`${OV_S_PREFIX}${rawCtxId}`]: reg.s,
      [`${OV_C_PREFIX}${rawCtxId}`]: reg.c,
      [`${OV_B_PREFIX}${rawCtxId}`]: reg.b,
    };
  }
  // Dead override: tombstone floor — only ov_c: with value max(S, C).
  return {
    [`${OV_C_PREFIX}${rawCtxId}`]: Math.max(reg.s, reg.c),
  };
}

// ---------------------------------------------------------------------------
// Structured parse result — separates decoded frontier IDs from override keys
// ---------------------------------------------------------------------------

/**
 * Structured result of parsing a `contexts` map.
 *
 * `frontiers` — Map from **raw** context ID to merged frontier value.
 *   Wire escaping (`esc:` prefix) is removed at decode time, so the same raw
 *   context ID is used in both maps.  Raw IDs may themselves begin with
 *   reserved prefixes such as `ov_*` or `esc:` — that is not an error.
 *
 * `overrides` — Map from **raw** context ID to validated `OverrideRegister`.
 *   Keyed by the same raw IDs as `frontiers`, so `isOverrideActive` can safely
 *   evaluate `overrides.get(rawCtx)` against `frontiers.get(rawCtx)`.
 */
export interface ParsedContexts {
  readonly frontiers: ReadonlyMap<string, number>;
  readonly overrides: ReadonlyMap<string, OverrideRegister>;
}

// ---------------------------------------------------------------------------
// Internal single decode path — the only place validation logic lives
// ---------------------------------------------------------------------------

/**
 * Decode and validate a raw `contexts` object in one pass.
 *
 * Returns three projections of the same validated data:
 *
 * - `wire`      — null-prototype `Record<string, number>` with `ov_*` keys
 *                 preserved in wire form (for `ReadStateBlob.contexts` compat).
 *                 Escaped frontier keys are stored as-is (e.g. `esc:ov_s:x`).
 *                 Built with `Object.create(null)` so that context IDs that
 *                 collide with `Object.prototype` properties (e.g. `__proto__`)
 *                 survive as own-property entries in both projections.
 * - `frontiers` — `Map<rawCtx, number>` — frontier keys unescaped; same raw
 *                 domain as `overrides`.
 * - `overrides` — `Map<rawCtx, OverrideRegister>` — registers keyed by the
 *                 raw context suffix (after the `ov_*:` prefix).
 *
 * Validation rules (spec NIP-RS §Content Validation):
 * 1. Bucket all `ov_*` entries by suffix.
 * 2. Validate each suffix as a whole group: complete live (S+C+B) or
 *    tombstone floor (C-only).  Any other shape drops the entire group;
 *    the frontier entry for that suffix is retained.
 * 3. Check 256-byte UTF-8 length on EVERY wire key — frontier and override
 *    siblings alike.  One oversized sibling drops its whole group; an
 *    oversized frontier key drops only that entry.
 * 4. Unescape frontier wire keys (`esc:…` → raw ctx) for `frontiers`.
 *
 * Exported so `parseReadStateEvent` can call once and derive both
 * `blob.contexts` and the structured `ParsedContexts` from one result.
 */
export function decodeContexts(raw: Record<string, unknown>): {
  wire: Record<string, number>;
  frontiers: Map<string, number>;
  overrides: Map<string, OverrideRegister>;
} {
  const enc = new TextEncoder();
  const isUint32 = (v: unknown): v is number =>
    typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= 4294967295;

  // ── Pass 1: bucket override candidates by suffix ─────────────────────────
  const sWire = new Map<string, string>(); // suffix → wire key (byte-length check)
  const cWire = new Map<string, string>();
  const bWire = new Map<string, string>();
  const sVal = new Map<string, unknown>();
  const cVal = new Map<string, unknown>();
  const bVal = new Map<string, unknown>();
  const frontierEntries: Array<[string, unknown]> = [];

  for (const [key, value] of Object.entries(raw)) {
    if (key.startsWith(OV_S_PREFIX)) {
      const suffix = key.slice(OV_S_PREFIX.length);
      sWire.set(suffix, key);
      sVal.set(suffix, value);
    } else if (key.startsWith(OV_C_PREFIX)) {
      const suffix = key.slice(OV_C_PREFIX.length);
      cWire.set(suffix, key);
      cVal.set(suffix, value);
    } else if (key.startsWith(OV_B_PREFIX)) {
      const suffix = key.slice(OV_B_PREFIX.length);
      bWire.set(suffix, key);
      bVal.set(suffix, value);
    } else {
      frontierEntries.push([key, value]);
    }
  }

  // ── Pass 2: validate each suffix as a group ───────────────────────────────
  const overrides = new Map<string, OverrideRegister>();
  // Use a null-prototype object so context IDs that coincide with
  // Object.prototype property names (e.g. `__proto__`, `toString`) survive
  // as own-property entries rather than silently colliding.
  const wire: Record<string, number> = Object.create(null) as Record<
    string,
    number
  >;
  const suffixes = new Set([...sWire.keys(), ...cWire.keys(), ...bWire.keys()]);

  for (const suffix of suffixes) {
    const hasS = sWire.has(suffix);
    const hasC = cWire.has(suffix);
    const hasB = bWire.has(suffix);
    const sv = sVal.get(suffix);
    const cv = cVal.get(suffix);
    const bv = bVal.get(suffix);

    if (!hasS && hasC && !hasB) {
      // Tombstone floor: only ov_c: present.
      const cwk = cWire.get(suffix) ?? "";
      if (enc.encode(cwk).length > 256) continue; // key too long → drop group
      if (!isUint32(cv)) continue;
      overrides.set(suffix, { s: 0, c: cv, b: 0 });
      wire[cwk] = cv;
    } else if (hasS && hasC && hasB) {
      // Complete live group: all three sibling wire keys must be ≤ 256 bytes.
      const swk = sWire.get(suffix) ?? "";
      const cwk = cWire.get(suffix) ?? "";
      const bwk = bWire.get(suffix) ?? "";
      if (
        enc.encode(swk).length > 256 ||
        enc.encode(cwk).length > 256 ||
        enc.encode(bwk).length > 256
      )
        continue; // any sibling key too long → drop group
      if (!isUint32(sv) || !isUint32(cv) || !isUint32(bv)) continue;
      overrides.set(suffix, { s: sv, c: cv, b: bv });
      wire[swk] = sv;
      wire[cwk] = cv;
      wire[bwk] = bv;
    }
    // Any other shape (B-only, S-only, S+B, C+B, S+C): silently dropped.
    // The frontier entry for `suffix` is retained in frontierEntries.
  }

  // ── Pass 3: validate frontier entries, unescape for structured map ────────
  const frontiers = new Map<string, number>();
  for (const [wireKey, value] of frontierEntries) {
    if (enc.encode(wireKey).length > 256) continue;
    if (!isUint32(value)) continue;
    // Flat wire map preserves the wire key (esc:… stays as-is for the manager).
    wire[wireKey] = value;
    // Structured frontiers map uses the raw context ID.
    const rawCtx = unescapeFrontierKey(wireKey);
    // max-merge in case a malformed blob has both escaped and unescaped forms.
    const existing = frontiers.get(rawCtx);
    if (existing === undefined || value > existing) {
      frontiers.set(rawCtx, value);
    }
  }

  return { wire, frontiers, overrides };
}

/**
 * Parse and validate a raw `contexts` object from a decoded blob into
 * separate frontier and override namespaces.
 *
 * This is the structured decode entry point for slices 2–3.  It delegates
 * to the internal `decodeContexts` core and returns the structured projection.
 * See `decodeContexts` for the full validation spec.
 */
export function parseContexts(raw: Record<string, unknown>): ParsedContexts {
  const { frontiers, overrides } = decodeContexts(raw);
  return { frontiers, overrides };
}

// ---------------------------------------------------------------------------
// Core validation and sanitization
// ---------------------------------------------------------------------------

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

/**
 * Sanitize and validate a raw contexts object from a decoded blob into the
 * flat `Record<string, number>` stored in `ReadStateBlob.contexts`.
 *
 * This is the backward-compatible path for `readStateManager`, which reads
 * the flat blob directly.  Override entries (`ov_*` keys) are validated as
 * complete logical groups BEFORE any per-entry processing; partial groups are
 * dropped while their frontier entry is retained.  The 256-byte UTF-8
 * key-length check is applied to every wire key — including override siblings.
 * Escaped frontier keys (`esc:…`) are stored as-is in the flat map (the
 * structured path via `parseContexts` applies unescaping; this flat path
 * preserves the wire form for the manager).
 *
 * Thin projection of the internal `decodeContexts` core — validation logic
 * lives exactly once.
 */
export function sanitizeContexts(
  contexts: Record<string, unknown>,
): Record<string, number> {
  return decodeContexts(contexts).wire;
}

// Exactly 32 lowercase hexadecimal characters as required by NIP-RS :55/:68.
const READ_STATE_SLOT_ID_PATTERN = /^[0-9a-f]{32}$/;

export function isValidReadStateDTag(
  value: string | undefined,
): value is string {
  if (!value?.startsWith(READ_STATE_D_TAG_PREFIX)) return false;
  const slotId = value.slice(READ_STATE_D_TAG_PREFIX.length);
  return READ_STATE_SLOT_ID_PATTERN.test(slotId);
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
