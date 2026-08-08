/**
 * User-configurable reply mention settings ("reply auto-mention").
 *
 * The desktop composer folds the replied-to author into a reply's `p` tags so
 * they get woken even without a literal "@mention" in the body. This module is
 * the per-account persisted model that makes that behavior configurable:
 *
 * - `autoMentionRepliedTo` — toggle the implicit replied-to author `p` tag.
 * - `mentionPrefixPubkeys` — extra pubkeys always folded into a reply's
 *   `p` tags (a user-defined "reply prefix", e.g. a teammate or agent group
 *   that should be woken on every reply the user sends).
 *
 * Storage follows the notifications pattern: one localStorage key per account
 * (`<key>:<pubkey>`), sanitized on read so a corrupt hand-edit can never wedge
 * the composer. Kept React-free so the send paths (`hooks.ts`) and unit tests
 * can use it directly; the `useReplyMentionSettings` hook lives in
 * `../useReplyMentionSettings.ts`.
 */

export type ReplyMentionSettings = {
  /** When true (default), replying p-tags the replied-to event's author. */
  autoMentionRepliedTo: boolean;
  /** Extra pubkeys folded into every reply's p-tags. May be empty. */
  mentionPrefixPubkeys: string[];
};

export const DEFAULT_REPLY_MENTION_SETTINGS: ReplyMentionSettings = {
  autoMentionRepliedTo: true,
  mentionPrefixPubkeys: [],
};

const REPLY_MENTION_SETTINGS_STORAGE_KEY = "buzz-reply-mention-settings.v1";

const PUBKEY_HEX_PATTERN = /^[0-9a-f]{64}$/;

function sanitizeMentionPrefixPubkeys(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const seen = new Set<string>();
  const result: string[] = [];
  for (const item of value) {
    if (typeof item !== "string") {
      continue;
    }
    const lower = item.trim().toLowerCase();
    if (!PUBKEY_HEX_PATTERN.test(lower) || seen.has(lower)) {
      continue;
    }
    seen.add(lower);
    result.push(lower);
  }
  return result;
}

export function sanitizeReplyMentionSettings(
  value: unknown,
): ReplyMentionSettings {
  if (!value || typeof value !== "object") {
    return DEFAULT_REPLY_MENTION_SETTINGS;
  }

  const candidate = value as Partial<ReplyMentionSettings>;
  return {
    autoMentionRepliedTo:
      typeof candidate.autoMentionRepliedTo === "boolean"
        ? candidate.autoMentionRepliedTo
        : DEFAULT_REPLY_MENTION_SETTINGS.autoMentionRepliedTo,
    mentionPrefixPubkeys: sanitizeMentionPrefixPubkeys(
      candidate.mentionPrefixPubkeys,
    ),
  };
}

function replyMentionSettingsStorageKey(pubkey: string) {
  return `${REPLY_MENTION_SETTINGS_STORAGE_KEY}:${pubkey}`;
}

export function readStoredReplyMentionSettings(
  pubkey: string,
): ReplyMentionSettings {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return DEFAULT_REPLY_MENTION_SETTINGS;
  }

  const rawValue = window.localStorage.getItem(
    replyMentionSettingsStorageKey(pubkey),
  );
  if (!rawValue) {
    return DEFAULT_REPLY_MENTION_SETTINGS;
  }

  try {
    return sanitizeReplyMentionSettings(JSON.parse(rawValue));
  } catch {
    return DEFAULT_REPLY_MENTION_SETTINGS;
  }
}

export function writeStoredReplyMentionSettings(
  pubkey: string,
  settings: ReplyMentionSettings,
) {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return;
  }

  window.localStorage.setItem(
    replyMentionSettingsStorageKey(pubkey),
    JSON.stringify(settings),
  );
}
