import * as React from "react";

import { trimMapToSize } from "@/shared/lib/trimMapToSize";

export type DraftState = {
  content: string;
  selectionStart: number;
  selectionEnd: number;
};

const sharedDrafts = new Map<string, DraftState>();

export function clearAllDrafts(): void {
  sharedDrafts.clear();
}

export function useDrafts() {
  const saveDraft = React.useCallback(
    (channelId: string, draft: DraftState) => {
      if (draft.content.trim().length === 0) {
        return;
      }
      sharedDrafts.set(channelId, draft);
      trimMapToSize(sharedDrafts, 50);
    },
    [],
  );

  const loadDraft = React.useCallback(
    (channelId: string): DraftState | undefined => {
      return sharedDrafts.get(channelId);
    },
    [],
  );

  const clearDraft = React.useCallback((channelId: string) => {
    sharedDrafts.delete(channelId);
  }, []);

  /** Save draft if content is non-empty, otherwise clear it. */
  const persistDraft = React.useCallback(
    (channelId: string, content: string) => {
      if (content.trim().length > 0) {
        saveDraft(channelId, {
          content,
          selectionEnd: content.length,
          selectionStart: content.length,
        });
      } else {
        clearDraft(channelId);
      }
    },
    [saveDraft, clearDraft],
  );

  return { saveDraft, loadDraft, clearDraft, persistDraft };
}

export type UseDraftsResult = ReturnType<typeof useDrafts>;
