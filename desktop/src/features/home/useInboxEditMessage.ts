import * as React from "react";

import { useEditMessageMutation } from "@/features/messages/hooks";
import type { Channel } from "@/shared/api/types";

export type InboxEditMessageInput = {
  content: string;
  eventId: string;
  mediaTags?: string[][];
  mentionPubkeys?: string[];
};

/** Publishes an Inbox edit, then refreshes its structural event overlay. */
export function useInboxEditMessage(
  channel: Channel | null,
  refreshStructuralEvents: () => Promise<void>,
) {
  const editMessageMutation = useEditMessageMutation(channel);
  const mutateRef = React.useRef(editMessageMutation.mutateAsync);
  const refreshRef = React.useRef(refreshStructuralEvents);
  mutateRef.current = editMessageMutation.mutateAsync;
  refreshRef.current = refreshStructuralEvents;

  const editMessage = React.useCallback(
    async (input: InboxEditMessageInput) => {
      await mutateRef.current(input);
      await refreshRef.current();
    },
    [],
  );

  return {
    editMessage,
    isEditingMessage: editMessageMutation.isPending,
  } as const;
}
