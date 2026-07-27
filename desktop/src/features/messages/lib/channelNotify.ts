/**
 * Compose-side helpers for channel-wide mentions (`@channel` / `@here`).
 *
 * `channel` and `here` are reserved mention tokens: they never resolve to a
 * member pubkey, even when somebody's display name is literally "here". What
 * they do produce is a single `["notify", mode]` tag on the outgoing event.
 */

import {
  NOTIFY_MODES,
  NOTIFY_TAG,
  type NotifyMode,
} from "@/shared/constants/notify";
import { hasMention } from "./hasMention";

export type { NotifyMode };

/**
 * Resolve a display name to the notify mode it reserves, if any. Matching is
 * exact and case-insensitive: `@Channel` is reserved, `@channels` is not.
 */
export function reservedMentionToken(name: string): NotifyMode | null {
  const normalized = name.trim().toLowerCase();
  return NOTIFY_MODES.find((mode) => mode === normalized) ?? null;
}

/** Whether `name` is a reserved mention token and so never maps to a pubkey. */
export function isReservedMentionName(name: string): boolean {
  return reservedMentionToken(name) !== null;
}

/**
 * Detect the notify mode an outgoing message body asks for, or null.
 *
 * Uses the shared `@mention` matcher, so tokens inside code fences, indented
 * blocks, or backtick spans are masked and never notify. `@channel` wins over
 * `@here` when both appear: it is the broader audience, so confirming it also
 * covers everyone `@here` would have reached.
 */
export function detectNotifyMode(text: string): NotifyMode | null {
  return NOTIFY_MODES.find((mode) => hasMention(text, mode)) ?? null;
}

/** Outgoing tag set for a notify mode — empty when there is nothing to notify. */
export function buildNotifyTags(mode: NotifyMode | null): string[][] {
  return mode ? [[NOTIFY_TAG, mode]] : [];
}
