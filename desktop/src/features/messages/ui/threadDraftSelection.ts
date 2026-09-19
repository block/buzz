import * as React from "react";
import type { DraftState } from "../lib/useDrafts";

/** Keep legacy drafts separate; never rename over another root draft. */
export function resolveThreadDraftKey({
  rootId,
  channelId,
  requestedKeys,
  messageIds,
  loadDraft,
}: {
  rootId: string;
  channelId: string;
  requestedKeys: readonly (string | null | undefined)[];
  messageIds: ReadonlySet<string>;
  loadDraft: (key: string) => DraftState | undefined;
}) {
  for (const key of requestedKeys) {
    if (!key?.startsWith("thread:")) continue;
    const headId = key.slice("thread:".length);
    const draft = loadDraft(key);
    if (
      draft?.channelId === channelId &&
      (headId === rootId || messageIds.has(headId))
    )
      return key;
  }
  return `thread:${rootId}`;
}

/** Restore once per draft visit; later explicit selections retain authority. */
export function useThreadDraftSelection({
  draftKey,
  rootId,
  parentTargetId,
  loadDraft,
}: {
  draftKey: string;
  rootId: string;
  parentTargetId: string | null;
  loadDraft: (key: string) => DraftState | undefined;
}) {
  const parentId = parentTargetId === rootId ? null : parentTargetId;
  const restore = () => {
    if (parentId) return parentId;
    const saved = loadDraft(draftKey);
    const legacyHeadId = draftKey.slice("thread:".length);
    return saved?.replyContextId !== undefined
      ? saved.replyContextId
      : (parentId ?? (legacyHeadId !== rootId ? legacyHeadId : null));
  };
  const [selection, setSelection] = React.useState(() => ({
    draftKey,
    parentId,
    contextId: restore(),
  }));
  const current =
    selection.draftKey !== draftKey
      ? { draftKey, parentId, contextId: restore() }
      : selection.parentId !== parentId
        ? { draftKey, parentId, contextId: parentId }
        : selection;
  if (current !== selection) setSelection(current);
  const setContextId = React.useCallback((contextId: string | null) => {
    setSelection((value) => ({ ...value, contextId }));
  }, []);
  return { contextId: current.contextId ?? null, setContextId };
}
