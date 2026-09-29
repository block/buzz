import * as React from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  loadDraftEntry,
  saveDraftEntry,
  type DraftState,
} from "@/features/messages/lib/useDrafts";

export type UserSignerDraftMentionRef = {
  displayName: string;
  pubkey: string;
  isAgent: boolean;
};

export type UserSignerDraftPayload = {
  channelId: string;
  draftKey: string;
  content: string;
  threadId?: string | null;
  requestId: string;
  /** Mentions from buzz_draft_message — required for composer chips. */
  mentionRefs?: UserSignerDraftMentionRef[] | null;
};

/**
 * Listen for Desktop user-signer `draft_message` IPC results and materialize
 * a composer draft. Never auto-sends — JM clicks Send.
 */
export function useUserSignerDraftListener(enabled = true): void {
  const { goChannel } = useAppNavigation();

  React.useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<UserSignerDraftPayload>("user-signer-draft", (event) => {
      const payload = event.payload;
      if (!payload?.channelId || !payload?.draftKey || !payload?.content) {
        return;
      }
      const now = new Date().toISOString();
      const existing = loadDraftEntry(payload.draftKey);
      const content =
        existing?.content && existing.content.trim().length > 0
          ? `${existing.content.trimEnd()}\n\n${payload.content}`
          : payload.content;
      const incomingRefs = (payload.mentionRefs ?? [])
        .filter(
          (ref) =>
            typeof ref?.displayName === "string" &&
            ref.displayName.trim() &&
            typeof ref?.pubkey === "string" &&
            ref.pubkey.trim(),
        )
        .map((ref) => ({
          displayName: ref.displayName.trim(),
          pubkey: ref.pubkey.trim(),
          isAgent: ref.isAgent !== false,
        }));
      const existingRefs = existing?.mentionRefs ?? [];
      const byPubkey = new Map(
        existingRefs.map((ref) => [ref.pubkey.toLowerCase(), ref]),
      );
      for (const ref of incomingRefs) {
        byPubkey.set(ref.pubkey.toLowerCase(), ref);
      }
      const mentionRefs = [...byPubkey.values()];
      const draft: DraftState = {
        channelId: payload.channelId,
        content,
        createdAt: existing?.createdAt ?? now,
        mentionRefs,
        pendingImeta: existing?.pendingImeta ?? [],
        selectionEnd: content.length,
        selectionStart: content.length,
        spoileredAttachmentUrls: existing?.spoileredAttachmentUrls ?? [],
        status: "active",
        updatedAt: now,
      };
      saveDraftEntry(payload.draftKey, draft);
      const thread = payload.threadId?.trim() || undefined;
      void goChannel(payload.channelId, {
        ...(thread ? { thread } : {}),
        force: true,
      });
      toast.message("Buzz Term draft ready — review and click Send", {
        description: thread ? `Thread draft in channel` : `Channel draft`,
      });
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [enabled, goChannel]);
}
