import * as React from "react";

import {
  loadActiveWorkspaceId,
  loadWorkspaces,
} from "@/features/workspaces/workspaceStorage";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

const QUICK_REACTION_STORAGE_KEY = "buzz.quick-reaction-emojis.v1";
const DEFAULT_QUICK_REACTIONS = ["👍", "❤️", "😂", "🎉"] as const;
const MAX_STORED_REACTIONS = 24;
const sessionQuickReactionEmojis = new Map<string, string[]>();

type QuickReactionEntry = {
  count: number;
  emoji: string;
  lastUsedAt: number;
};

function canUseLocalStorage() {
  if (typeof window === "undefined") return false;

  try {
    return Boolean(window.localStorage);
  } catch {
    return false;
  }
}

function getActiveWorkspaceScope() {
  if (!canUseLocalStorage()) return null;

  try {
    return loadActiveWorkspaceId() ?? loadWorkspaces()[0]?.id ?? null;
  } catch {
    return null;
  }
}

function quickReactionStorageKey(workspaceScope: string | null) {
  return workspaceScope
    ? `${QUICK_REACTION_STORAGE_KEY}:${workspaceScope}`
    : QUICK_REACTION_STORAGE_KEY;
}

function quickReactionSessionKey(
  limit: number,
  workspaceScope: string | null,
  customEmojiSignature: string,
) {
  return `${workspaceScope ?? "global"}:${customEmojiSignature}:${limit}`;
}

function normalizeEntry(entry: unknown): QuickReactionEntry | null {
  if (!entry || typeof entry !== "object") return null;

  const candidate = entry as Partial<QuickReactionEntry>;
  if (
    typeof candidate.emoji !== "string" ||
    candidate.emoji.trim().length === 0
  ) {
    return null;
  }

  return {
    count: Math.max(1, Math.floor(Number(candidate.count) || 1)),
    emoji: candidate.emoji,
    lastUsedAt: Math.max(0, Number(candidate.lastUsedAt) || 0),
  };
}

function sortEntries(entries: QuickReactionEntry[]) {
  return [...entries].sort((left, right) => {
    const countDelta = right.count - left.count;
    if (countDelta !== 0) return countDelta;
    return right.lastUsedAt - left.lastUsedAt;
  });
}

function readQuickReactionEntries(storageKey: string) {
  if (!canUseLocalStorage()) return [];

  try {
    const raw = window.localStorage.getItem(storageKey);
    const parsed = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(parsed)) return [];
    return sortEntries(
      parsed
        .map((entry) => normalizeEntry(entry))
        .filter((entry): entry is QuickReactionEntry => entry !== null),
    );
  } catch {
    return [];
  }
}

function writeQuickReactionEntries(
  entries: QuickReactionEntry[],
  storageKey: string,
) {
  if (!canUseLocalStorage()) return;

  try {
    window.localStorage.setItem(
      storageKey,
      JSON.stringify(sortEntries(entries).slice(0, MAX_STORED_REACTIONS)),
    );
  } catch {
    // Ignore storage failures; the reaction itself should still work.
  }
}

function customEmojiSignature(customEmoji: ReadonlyArray<CustomEmoji>) {
  return customEmoji
    .map((emoji) => emoji.shortcode.toLowerCase())
    .sort()
    .join(",");
}

function customEmojiShortcodesFromSignature(signature: string) {
  return new Set(signature ? signature.split(",") : []);
}

function isCustomEmojiShortcode(emoji: string) {
  return emoji.startsWith(":") && emoji.endsWith(":");
}

function canRenderQuickReactionEmoji(
  emoji: string,
  customEmojiShortcodes: ReadonlySet<string>,
) {
  if (!isCustomEmojiShortcode(emoji)) return true;
  return customEmojiShortcodes.has(emoji.slice(1, -1).toLowerCase());
}

function resolveQuickReactionEmojisWithShortcodes(
  entries: ReadonlyArray<Pick<QuickReactionEntry, "emoji">>,
  limit: number,
  customEmojiShortcodes: ReadonlySet<string>,
) {
  const seen = new Set<string>();
  const next: string[] = [];

  for (const entry of entries) {
    if (seen.has(entry.emoji)) continue;
    if (!canRenderQuickReactionEmoji(entry.emoji, customEmojiShortcodes)) {
      continue;
    }
    seen.add(entry.emoji);
    next.push(entry.emoji);
    if (next.length >= limit) return next;
  }

  for (const emoji of DEFAULT_QUICK_REACTIONS) {
    if (seen.has(emoji)) continue;
    seen.add(emoji);
    next.push(emoji);
    if (next.length >= limit) return next;
  }

  return next;
}

export function resolveQuickReactionEmojis(
  entries: ReadonlyArray<Pick<QuickReactionEntry, "emoji">>,
  limit: number,
  customEmoji: ReadonlyArray<CustomEmoji> = [],
) {
  return resolveQuickReactionEmojisWithShortcodes(
    entries,
    limit,
    customEmojiShortcodesFromSignature(customEmojiSignature(customEmoji)),
  );
}

function getQuickReactionEmojis(
  limit: number,
  workspaceScope: string | null,
  customEmojiSignature: string,
) {
  return resolveQuickReactionEmojisWithShortcodes(
    readQuickReactionEntries(quickReactionStorageKey(workspaceScope)),
    limit,
    customEmojiShortcodesFromSignature(customEmojiSignature),
  );
}

function getSessionQuickReactionEmojis(
  limit: number,
  workspaceScope: string | null,
  customEmojiSignature: string,
) {
  const sessionKey = quickReactionSessionKey(
    limit,
    workspaceScope,
    customEmojiSignature,
  );
  const cached = sessionQuickReactionEmojis.get(sessionKey);
  if (cached) return cached;

  const emojis = getQuickReactionEmojis(
    limit,
    workspaceScope,
    customEmojiSignature,
  );
  sessionQuickReactionEmojis.set(sessionKey, emojis);
  return emojis;
}

export function recordQuickReactionEmoji(emoji: string) {
  const trimmed = emoji.trim();
  if (!trimmed) return;

  const workspaceScope = getActiveWorkspaceScope();
  const storageKey = quickReactionStorageKey(workspaceScope);
  const entries = readQuickReactionEntries(storageKey);
  const existing = entries.find((entry) => entry.emoji === trimmed);
  if (existing) {
    existing.count += 1;
    existing.lastUsedAt = Date.now();
  } else {
    entries.push({
      count: 1,
      emoji: trimmed,
      lastUsedAt: Date.now(),
    });
  }

  // Keep the current hover tray stable; the stored recents apply on reload or
  // when another tab updates this workspace's quick reactions.
  writeQuickReactionEntries(entries, storageKey);
}

export function useQuickReactionEmojis(
  limit = 4,
  customEmoji: ReadonlyArray<CustomEmoji> = [],
) {
  const workspaceScope = getActiveWorkspaceScope();
  const customEmojiCacheKey = customEmojiSignature(customEmoji);
  const [emojis, setEmojis] = React.useState(() =>
    getSessionQuickReactionEmojis(limit, workspaceScope, customEmojiCacheKey),
  );

  React.useEffect(() => {
    if (typeof window === "undefined") return;

    const storageKey = quickReactionStorageKey(workspaceScope);
    const sessionKey = quickReactionSessionKey(
      limit,
      workspaceScope,
      customEmojiCacheKey,
    );
    const handleStorage = (event: StorageEvent) => {
      if (event.key === storageKey) {
        sessionQuickReactionEmojis.delete(sessionKey);
        setEmojis(
          getSessionQuickReactionEmojis(
            limit,
            workspaceScope,
            customEmojiCacheKey,
          ),
        );
      }
    };

    window.addEventListener("storage", handleStorage);
    setEmojis(
      getSessionQuickReactionEmojis(limit, workspaceScope, customEmojiCacheKey),
    );

    return () => {
      window.removeEventListener("storage", handleStorage);
    };
  }, [customEmojiCacheKey, limit, workspaceScope]);

  return emojis;
}
