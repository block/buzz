/**
 * `@channel` and `@here` — the tag convention, the permission gate, and who
 * each one actually notifies.
 *
 * Pure and dependency-free, in a `.mjs` sibling so `node:test` exercises the
 * exact source the app runs (same rationale as `applyEditTagOverlay.mjs`).
 *
 * Design notes, because the shape here is deliberate:
 *
 * A global mention is **one marker tag**, not N `p` tags. Writing a `p` tag per
 * member would bloat every event, leak the member list into the event, and —
 * worse — freeze the audience at send time. Someone who joins the channel an
 * hour later should still see that the announcement was addressed to everyone.
 * So the marker is resolved against *current* membership at read time.
 *
 * The permission gate is **soft**. Nostr events are signed client-side and no
 * server refuses to publish, so this stops the composer from offering a scope
 * the sender may not use, and lets readers mark an unauthorised one rather than
 * honour it. A modified client could still send one. That is the same
 * cooperative model deletion already relies on, and it should not be described
 * to users as enforcement.
 */

/** Tag name. Distinct from the existing `broadcast` tag, which means something
 * else entirely: a thread reply echoed into the main channel timeline. */
export const MENTION_SCOPE_TAG = "mention-scope";

export const MENTION_SCOPE_CHANNEL = "channel";
export const MENTION_SCOPE_HERE = "here";

const VALID_SCOPES = new Set([MENTION_SCOPE_CHANNEL, MENTION_SCOPE_HERE]);

/**
 * Find a global mention written in the message text.
 *
 * Deliberately literal: `@channel` and `@here` are matched in the composed
 * content rather than inserted through the mention autocomplete. That hook
 * carries debouncing, personas and teams, and threading two synthetic entries
 * through it is a much larger change than this feature needs to start working.
 * The cost is discoverability — nothing offers the words to you — so the
 * autocomplete entries remain worth adding later.
 *
 * `@channel` wins when both appear: it is the strictly wider audience, so
 * resolving the ambiguity the other way could silently drop people the author
 * plainly meant to reach.
 *
 * Both edges are guarded so an address or a path does not page everyone by
 * accident — the expensive false positive here is silently notifying forty
 * people. The trailing guard is `(?![\w-])` rather than `\b`, because `\b`
 * treats a hyphen as a boundary and would have matched `@here-ish` and
 * `@channel-ops`. A trailing `.` or `,` still counts as the end of the word,
 * so "read this @here." works.
 */
export function detectMentionScope(content) {
  const text = String(content ?? "");
  if (/(^|[^\w@])@channel(?![\w-])/i.test(text)) return MENTION_SCOPE_CHANNEL;
  if (/(^|[^\w@])@here(?![\w-])/i.test(text)) return MENTION_SCOPE_HERE;
  return null;
}

/** Read the scope marker off an event's tags, or null if there is none. */
export function mentionScopeOf(tags) {
  for (const tag of tags ?? []) {
    if (tag?.[0] !== MENTION_SCOPE_TAG) continue;
    const scope = tag[1];
    if (VALID_SCOPES.has(scope)) return scope;
  }
  return null;
}

/**
 * Should this member be notified by an event carrying `scope`?
 *
 * The author is never notified of their own broadcast.
 *
 * Mute handling is the one genuinely contested rule, so it is explicit here:
 * `@channel` pierces a muted channel because it exists for the things you
 * would want pulled out of a mute, and `@here` never does. A per-user opt-out
 * (`allowChannelMentionWhileMuted: false`) restores silence for anyone who
 * disagrees — the same escape hatch WhatsApp shipped, and the reason its
 * mute-piercing is tolerable rather than resented.
 */
export function shouldNotifyForMentionScope({
  scope,
  isAuthor = false,
  isMember = true,
  isMuted = false,
  presence = "online",
  allowChannelMentionWhileMuted = true,
}) {
  if (!VALID_SCOPES.has(scope)) return false;
  if (isAuthor) return false;
  if (!isMember) return false;

  if (scope === MENTION_SCOPE_HERE) {
    // "Around right now" means actually at the desk. `away` is deliberately
    // excluded: notifying someone who stepped out makes `@here` indistinguishable
    // from `@channel`, which is the whole distinction being drawn.
    if (presence !== "online") return false;
    return !isMuted;
  }

  if (isMuted) return allowChannelMentionWhileMuted;
  return true;
}
