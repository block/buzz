import * as React from "react";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import {
  type DraftMentionRef,
  loadDraftEntry,
  persistDraftEntry,
  subscribeToStore,
} from "@/features/messages/lib/useDrafts";

type ForumDraftSnapshot = {
  content: string;
  pendingImeta: ImetaMedia[];
  mentionRefs: DraftMentionRef[];
};

type Params = {
  draftKey: string | null | undefined;
  channelId: string | null;
  getComposerRevision: () => number;
  isEmpty: () => boolean;
  restore: (snapshot: ForumDraftSnapshot) => void;
};

/** Source-key recovery survives a visit; only a pristine visit follows storage. */
export function useForumDraftRecovery(params: Params) {
  const live = React.useRef(params);
  live.current = params;
  const mounted = React.useRef(false);
  const { draftKey, getComposerRevision } = params;
  React.useLayoutEffect(() => {
    mounted.current = true;
    const revision = getComposerRevision();
    let observed = draftKey ? loadDraftEntry(draftKey) : undefined;
    const unsubscribe = subscribeToStore(() => {
      if (!draftKey) return;
      const saved = loadDraftEntry(draftKey);
      if (saved === observed) return;
      observed = saved;
      // Local authored edits (including edit -> delete and attachment removal)
      // and new sends revoke this visit's right to adopt a background recovery.
      if (
        saved &&
        getComposerRevision() === revision &&
        live.current.isEmpty()
      ) {
        live.current.restore({
          ...saved,
          mentionRefs: saved.mentionRefs ?? [],
        });
      }
    });
    return () => {
      mounted.current = false;
      unsubscribe();
    };
  }, [draftKey, getComposerRevision]);

  return React.useCallback((snapshot: ForumDraftSnapshot) => {
    const source = live.current;
    const revision = source.getComposerRevision();
    return () => {
      if (source.getComposerRevision() !== revision) return;
      if (source.draftKey) {
        const existing = loadDraftEntry(source.draftKey);
        // Persistence is not authored intent. Still do not replace a different
        // stored payload from another owner, even if its revision is unchanged.
        if (
          existing &&
          (existing.content !== snapshot.content ||
            existing.channelId !== (source.channelId ?? source.draftKey) ||
            JSON.stringify(existing.pendingImeta) !==
              JSON.stringify(snapshot.pendingImeta) ||
            JSON.stringify(existing.mentionRefs ?? []) !==
              JSON.stringify(snapshot.mentionRefs) ||
            existing.spoileredAttachmentUrls.length > 0)
        )
          return;
        persistDraftEntry(
          source.draftKey,
          snapshot.content,
          source.channelId ?? source.draftKey,
          snapshot.pendingImeta,
          [],
          snapshot.mentionRefs,
        );
      }
      // Never call the departed editor. The current source visit above follows
      // durable recovery independently; a different thread never receives it.
      if (mounted.current && live.current.isEmpty())
        live.current.restore(snapshot);
    };
  }, []);
}
