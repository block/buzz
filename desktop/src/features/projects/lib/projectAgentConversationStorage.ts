const CONVERSATION_STORAGE_PREFIX = "buzz.projects.agentConversation";

/**
 * Minimal workspace-scoped pointer to the last inline Projects conversation.
 * `visibleAfter` (epoch seconds) anchors the thread to the first Projects
 * prompt — messages the reused DM channel held before that instant must
 * never render on the Projects page.
 */
export type StoredProjectsAgentConversation = {
  agentPubkey: string;
  channelId: string;
  visibleAfter: number;
};

function scopedKey(prefix: string, workspaceId: string) {
  return `${prefix}.${encodeURIComponent(workspaceId)}`;
}

/** Reads the last inline Projects conversation without persisting its content. */
export function readStoredProjectsAgentConversation(
  workspaceId: string | null,
): StoredProjectsAgentConversation | null {
  if (!workspaceId) return null;
  try {
    const raw = globalThis.localStorage?.getItem(
      scopedKey(CONVERSATION_STORAGE_PREFIX, workspaceId),
    );
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<StoredProjectsAgentConversation>;
    if (
      typeof value.agentPubkey !== "string" ||
      value.agentPubkey.length === 0 ||
      typeof value.channelId !== "string" ||
      value.channelId.length === 0 ||
      typeof value.visibleAfter !== "number" ||
      !Number.isFinite(value.visibleAfter) ||
      // A zero/negative cutoff would restore the DM's full history
      // (pointers written before the cutoff was prompt-anchored).
      value.visibleAfter <= 0
    ) {
      return null;
    }
    return {
      agentPubkey: value.agentPubkey,
      channelId: value.channelId,
      visibleAfter: value.visibleAfter,
    };
  } catch {
    return null;
  }
}

/** Saves only the channel pointer needed to restore the Projects conversation. */
export function writeStoredProjectsAgentConversation(
  workspaceId: string | null,
  conversation: StoredProjectsAgentConversation,
) {
  if (!workspaceId) return;
  try {
    globalThis.localStorage?.setItem(
      scopedKey(CONVERSATION_STORAGE_PREFIX, workspaceId),
      JSON.stringify(conversation),
    );
  } catch {
    // Persistence is best-effort; the in-memory conversation remains usable.
  }
}

/** Deletes the saved pointer so no prior conversation is restored. */
export function clearStoredProjectsAgentConversation(
  workspaceId: string | null,
) {
  if (!workspaceId) return;
  try {
    globalThis.localStorage?.removeItem(
      scopedKey(CONVERSATION_STORAGE_PREFIX, workspaceId),
    );
  } catch {
    // Persistence is best-effort; the current page still clears immediately.
  }
}
