import * as React from "react";

import { imetaMediaFromTags } from "@/features/messages/lib/imetaMediaMarkdown";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Builds a reference-stable editTarget object and open-thread follow handlers
 * for ChannelPane. Inline object/arrow literals previously defeated ChannelPane's
 * React.memo whenever ChannelScreen re-rendered (including typing churn).
 */
export function useChannelPaneEditAndFollow({
  editTargetId,
  effectiveOpenThreadHeadId,
  followThread,
  isNotifiedForEffectiveThread,
  timelineMessages,
  unfollowThread,
}: {
  editTargetId: string | null;
  effectiveOpenThreadHeadId: string | null;
  followThread: (rootId: string) => void;
  isNotifiedForEffectiveThread: boolean;
  timelineMessages: TimelineMessage[];
  unfollowThread: (rootId: string) => void;
}): {
  editTarget: {
    author: string;
    body: string;
    id: string;
    imetaMedia: ReturnType<typeof imetaMediaFromTags>;
  } | null;
  editTargetMessage: TimelineMessage | null;
  onFollowThread: (() => void) | undefined;
  onUnfollowThread: (() => void) | undefined;
} {
  const editTargetMessage = React.useMemo(
    () =>
      timelineMessages.find((message) => message.id === editTargetId) ?? null,
    [editTargetId, timelineMessages],
  );
  const editTarget = React.useMemo(
    () =>
      editTargetMessage
        ? {
            author: editTargetMessage.author,
            body: editTargetMessage.body,
            id: editTargetMessage.id,
            imetaMedia: imetaMediaFromTags(editTargetMessage.tags),
          }
        : null,
    [editTargetMessage],
  );
  const handleFollowOpenThread = React.useCallback(() => {
    if (effectiveOpenThreadHeadId != null) {
      followThread(effectiveOpenThreadHeadId);
    }
  }, [effectiveOpenThreadHeadId, followThread]);
  const handleUnfollowOpenThread = React.useCallback(() => {
    if (effectiveOpenThreadHeadId != null) {
      unfollowThread(effectiveOpenThreadHeadId);
    }
  }, [effectiveOpenThreadHeadId, unfollowThread]);
  const onFollowThread =
    effectiveOpenThreadHeadId != null && !isNotifiedForEffectiveThread
      ? handleFollowOpenThread
      : undefined;
  const onUnfollowThread =
    effectiveOpenThreadHeadId != null && isNotifiedForEffectiveThread
      ? handleUnfollowOpenThread
      : undefined;
  return {
    editTarget,
    editTargetMessage,
    onFollowThread,
    onUnfollowThread,
  };
}
