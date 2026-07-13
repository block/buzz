const CONVERSATION_STORAGE_PREFIX = "buzz.projects.agentConversation";
const CLEARED_AT_STORAGE_PREFIX = "buzz.projects.agentConversationClearedAt";

/** Minimal workspace-scoped pointer to the last inline Projects conversation. */
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
      value.visibleAfter < 0
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

/** Reads the cutoff used to prevent cleared history from being restored. */
export function readProjectsAgentConversationClearedAt(
  workspaceId: string | null,
) {
  if (!workspaceId) return 0;
  try {
    const value = Number(
      globalThis.localStorage?.getItem(
        scopedKey(CLEARED_AT_STORAGE_PREFIX, workspaceId),
      ),
    );
    return Number.isFinite(value) && value > 0 ? value : 0;
  } catch {
    return 0;
  }
}

/** Clears the saved pointer and records when prior history was dismissed. */
export function markProjectsAgentConversationCleared(
  workspaceId: string | null,
  clearedAt: number,
) {
  if (!workspaceId) return;
  try {
    globalThis.localStorage?.removeItem(
      scopedKey(CONVERSATION_STORAGE_PREFIX, workspaceId),
    );
    globalThis.localStorage?.setItem(
      scopedKey(CLEARED_AT_STORAGE_PREFIX, workspaceId),
      String(clearedAt),
    );
  } catch {
    // Persistence is best-effort; the current page still clears immediately.
  }
}
