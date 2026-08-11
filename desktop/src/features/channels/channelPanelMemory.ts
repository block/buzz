/**
 * Per-channel memory of the thread panel, so returning to a channel restores
 * the panel the way it was left: open on the same thread, or closed.
 *
 * ChannelScreen records the current `?thread` search value continuously;
 * `goChannel` recalls it when building the URL for a navigation that carries
 * no explicit target, so the restored URL is right the first time — one
 * history entry per switch, and explicit targets (deep links, search hits,
 * mention clicks) win by construction.
 *
 * The memory is tri-state per channel: a thread head id ("open on this
 * thread"), `null` ("the user left it closed" — a closed panel must stay
 * closed on return), or no entry ("never visited this session").
 *
 * Session-scoped by design: backed by sessionStorage (the thread-panel width
 * precedent, `useThreadPanelWidth`) so it survives a reload but not an app
 * restart. Channel ids are community-local, so this module-level singleton is
 * community-scoped state and its reset is wired into `resetCommunityState()`
 * (`useCommunityInit.ts`).
 */

const CHANNEL_PANEL_MEMORY_SESSION_KEY = "buzz.channels.thread-panel-memory";

let memoryByChannelId: Map<string, string | null> | null = null;

function readStoredMemory(): Map<string, string | null> {
  if (typeof window === "undefined") {
    return new Map();
  }

  try {
    const raw = window.sessionStorage.getItem(CHANNEL_PANEL_MEMORY_SESSION_KEY);
    if (!raw) {
      return new Map();
    }

    const parsed: unknown = JSON.parse(raw);
    if (
      parsed === null ||
      typeof parsed !== "object" ||
      Array.isArray(parsed)
    ) {
      return new Map();
    }

    const entries = Object.entries(parsed).filter(
      (entry): entry is [string, string | null] =>
        typeof entry[1] === "string" || entry[1] === null,
    );
    return new Map(entries);
  } catch {
    return new Map();
  }
}

function memory(): Map<string, string | null> {
  if (!memoryByChannelId) {
    memoryByChannelId = readStoredMemory();
  }
  return memoryByChannelId;
}

function persistMemory(current: Map<string, string | null>): void {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.sessionStorage.setItem(
      CHANNEL_PANEL_MEMORY_SESSION_KEY,
      JSON.stringify(Object.fromEntries(current)),
    );
  } catch {
    // Persistence is best-effort; the in-memory map still applies.
  }
}

/**
 * Record the thread panel state currently showing in a channel.
 * `null` means the panel is closed.
 */
export function rememberChannelThread(
  channelId: string,
  threadHeadId: string | null,
): void {
  const current = memory();
  if (current.has(channelId) && current.get(channelId) === threadHeadId) {
    return;
  }

  current.set(channelId, threadHeadId);
  persistMemory(current);
}

/**
 * The thread panel state to restore when re-entering a channel: a thread head
 * id to reopen, `null` if the user left the panel closed, or `undefined` if
 * the channel has no memory this session.
 */
export function recallChannelThread(
  channelId: string,
): string | null | undefined {
  return memory().get(channelId);
}

/**
 * Forget every channel's panel state. Wired into `resetCommunityState()` —
 * channel ids are community-local, so remembered panel state must not leak
 * across a community switch.
 */
export function resetChannelPanelMemory(): void {
  memoryByChannelId = new Map();

  if (typeof window === "undefined") {
    return;
  }

  try {
    window.sessionStorage.removeItem(CHANNEL_PANEL_MEMORY_SESSION_KEY);
  } catch {
    // Best-effort; the in-memory map is already cleared.
  }
}
