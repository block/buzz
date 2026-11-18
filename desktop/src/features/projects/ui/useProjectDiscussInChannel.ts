import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  loadDraftEntry,
  saveDraftEntry,
} from "@/features/messages/lib/useDrafts";
import {
  mergeSelectionDiscussDraft,
  projectSelectionDiscussContent,
  type ProjectSelectionItem,
} from "@/features/projects/lib/projectSelection";

export function useProjectDiscussInChannel(items: ProjectSelectionItem[]) {
  const { goChannel } = useAppNavigation();

  return React.useCallback(
    (channelId: string) => {
      const now = new Date().toISOString();
      const existing = loadDraftEntry(channelId);
      const content = mergeSelectionDiscussDraft(
        existing?.content,
        projectSelectionDiscussContent(items),
      );
      saveDraftEntry(channelId, {
        channelId,
        content,
        createdAt: existing?.createdAt ?? now,
        mentionRefs: existing?.mentionRefs ?? [],
        pendingImeta: existing?.pendingImeta ?? [],
        selectionEnd: content.length,
        selectionStart: content.length,
        spoileredAttachmentUrls: existing?.spoileredAttachmentUrls ?? [],
        status: "active",
        updatedAt: now,
      });
      void goChannel(channelId);
    },
    [goChannel, items],
  );
}
