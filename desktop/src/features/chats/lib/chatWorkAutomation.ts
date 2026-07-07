import * as React from "react";

// Per-chat automation preferences for the work panel, plus watermarks that
// keep the auto-prompts from repeating (one CI nudge per failing head sha,
// one comment nudge per count increase). Local state: the prompts are sent
// from this client into the chat, so they never need to sync.
const STORAGE_PREFIX = "buzz:chat-work-automation:v1";
const STORAGE_EVENT = "buzz:chat-work-automation-changed";

/**
 * Tag attached to automation-generated prompts (auto-fix CI, address
 * comments). The message still reaches the agent like any user message, but
 * the chat timeline renders only its activity — not the message bubble — so
 * armed automation feels ambient instead of ventriloquized.
 */
// Rides the whitelisted ["client", ...] marker-tag channel: the imeta-only
// media path rejects any other prefix and fails the whole send.
export const CHAT_AUTOMATION_TAG: [string, string] = ["client", "automation"];

/** Tag for one automation prompt, carrying its kind for the marker row. */
export function chatAutomationTag(kind: "ci" | "comments"): string[] {
  return [...CHAT_AUTOMATION_TAG, kind];
}

/** Marker-row label for an automation message's tag. */
export function chatAutomationLabel(
  tag: readonly string[] | undefined,
  agentName: string,
) {
  if (tag?.[2] === "ci") {
    return `Asked ${agentName} to fix the CI failures`;
  }
  if (tag?.[2] === "comments") {
    return `Asked ${agentName} to address the review comments`;
  }
  return `Sent ${agentName} automation instructions`;
}

export type ChatWorkAutomation = {
  autoFixCi: boolean;
  addressComments: boolean;
  /** Head sha of the last CI failure the agent was asked to fix. */
  lastCiNudgeSha: string | null;
  /** Comment total at the last address-comments nudge. */
  lastCommentNudgeCount: number | null;
  /** Epoch ms of the last CI nudge — drives the persistent-failure re-nudge. */
  lastCiNudgeAt: number | null;
  /** Epoch ms of the last comment nudge. */
  lastCommentNudgeAt: number | null;
};

const DEFAULTS: ChatWorkAutomation = {
  autoFixCi: false,
  addressComments: false,
  lastCiNudgeSha: null,
  lastCommentNudgeCount: null,
  lastCiNudgeAt: null,
  lastCommentNudgeAt: null,
};

function storageKey(chatId: string) {
  return `${STORAGE_PREFIX}:${chatId}`;
}

export function readChatWorkAutomation(chatId: string): ChatWorkAutomation {
  if (typeof window === "undefined") {
    return DEFAULTS;
  }
  try {
    const raw = window.localStorage.getItem(storageKey(chatId));
    if (!raw) {
      return DEFAULTS;
    }
    const parsed = JSON.parse(raw) as Partial<ChatWorkAutomation>;
    return {
      autoFixCi: Boolean(parsed.autoFixCi),
      addressComments: Boolean(parsed.addressComments),
      lastCiNudgeSha:
        typeof parsed.lastCiNudgeSha === "string"
          ? parsed.lastCiNudgeSha
          : null,
      lastCommentNudgeCount:
        typeof parsed.lastCommentNudgeCount === "number"
          ? parsed.lastCommentNudgeCount
          : null,
      lastCiNudgeAt:
        typeof parsed.lastCiNudgeAt === "number" ? parsed.lastCiNudgeAt : null,
      lastCommentNudgeAt:
        typeof parsed.lastCommentNudgeAt === "number"
          ? parsed.lastCommentNudgeAt
          : null,
    };
  } catch {
    return DEFAULTS;
  }
}

export function updateChatWorkAutomation(
  chatId: string,
  patch: Partial<ChatWorkAutomation>,
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    const next = { ...readChatWorkAutomation(chatId), ...patch };
    window.localStorage.setItem(storageKey(chatId), JSON.stringify(next));
    window.dispatchEvent(new CustomEvent(STORAGE_EVENT));
  } catch {
    // Preferences are a convenience layer; ignore unavailable storage.
  }
}

export function useChatWorkAutomation(chatId: string): ChatWorkAutomation {
  const [state, setState] = React.useState<{
    chatId: string;
    automation: ChatWorkAutomation;
  }>(() => ({
    chatId,
    automation: readChatWorkAutomation(chatId),
  }));
  const automation =
    state.chatId === chatId ? state.automation : readChatWorkAutomation(chatId);

  React.useEffect(() => {
    // Content-compare: a fresh object per storage event would re-run every
    // consumer effect (and re-render every panel) even when nothing changed.
    const refresh = () =>
      setState((current) => {
        const next = readChatWorkAutomation(chatId);
        return current.chatId === chatId &&
          current.automation.autoFixCi === next.autoFixCi &&
          current.automation.addressComments === next.addressComments &&
          current.automation.lastCiNudgeSha === next.lastCiNudgeSha &&
          current.automation.lastCommentNudgeCount ===
            next.lastCommentNudgeCount &&
          current.automation.lastCiNudgeAt === next.lastCiNudgeAt &&
          current.automation.lastCommentNudgeAt === next.lastCommentNudgeAt
          ? current
          : { chatId, automation: next };
      });
    refresh();
    window.addEventListener(STORAGE_EVENT, refresh);
    window.addEventListener("storage", refresh);
    return () => {
      window.removeEventListener(STORAGE_EVENT, refresh);
      window.removeEventListener("storage", refresh);
    };
  }, [chatId]);

  return automation;
}

const PR_STORAGE_PREFIX = "buzz:chat-work-pr:v1";

/**
 * The PR pinned to a chat. Posted links age out of the windowed message
 * fetch and branch discovery can resolve several chats sharing a reused
 * worktree to the same PR — the pin keeps each chat on the PR it actually
 * resolved first, with posted links always overriding.
 */
export function readChatPinnedPr(chatId: string): ChatPinnedPr | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(`${PR_STORAGE_PREFIX}:${chatId}`);
    if (raw === null) {
      return null;
    }
    // Older entries stored the bare href string.
    if (!raw.startsWith("{")) {
      return { href: raw, manual: raw === CHAT_PR_UNPINNED };
    }
    const parsed = JSON.parse(raw) as Partial<ChatPinnedPr>;
    return typeof parsed.href === "string"
      ? { href: parsed.href, manual: Boolean(parsed.manual) }
      : null;
  } catch {
    return null;
  }
}

/**
 * Explicit "no PR" pin: suppresses branch discovery for the chat (the user
 * said the discovered PR was wrong) while posted links still override.
 */
export const CHAT_PR_UNPINNED = "";

export type ChatPinnedPr = {
  href: string;
  /**
   * True when the user pinned (or unlinked) explicitly — a manual pin
   * outranks every automatic source, including links posted in the chat.
   */
  manual: boolean;
};

export function writeChatPinnedPr(
  chatId: string,
  href: string,
  manual = false,
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(
      `${PR_STORAGE_PREFIX}:${chatId}`,
      JSON.stringify({ href, manual }),
    );
  } catch {
    // Best-effort.
  }
}

export function clearChatPinnedPr(chatId: string) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.removeItem(`${PR_STORAGE_PREFIX}:${chatId}`);
  } catch {
    // Best-effort.
  }
}
