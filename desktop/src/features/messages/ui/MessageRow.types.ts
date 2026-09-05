import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { VideoReviewContext } from "@/shared/ui/VideoPlayer";

export type ThreadDepthGuideAction = {
  active?: boolean;
  depth: number;
  label: string;
  message: TimelineMessage;
};

export type MessageRowProps = {
  channelId?: string | null;
  currentPubkey?: string;
  collapseDepthGuideActions?: ReadonlyArray<ThreadDepthGuideAction>;
  connectDescendants?: boolean;
  depthGuideDepths?: ReadonlyArray<number>;
  highlighted?: boolean;
  highlightDescendantRail?: boolean;
  highlightReplyConnector?: boolean;
  highlightThreadLineDepths?: ReadonlyArray<number>;
  hoverBackground?: boolean;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
  hideAgentAccessBadge?: boolean;
  actionBarPlacement?: "floating" | "inside";
  collapseDescendantsLabel?: string;
  isFollowingThread?: boolean;
  isContinuation?: boolean;
  isUnread?: boolean;
  layoutVariant?: "default" | "thread-reply";
  message: TimelineMessage;
  onCollapseDepthGuide?: (message: TimelineMessage) => void;
  onCollapseDepthGuideHoverChange?: (
    message: TimelineMessage,
    hovered: boolean,
  ) => void;
  onCollapseDescendants?: (message: TimelineMessage) => void;
  onCollapseDescendantsHoverChange?: (
    message: TimelineMessage,
    hovered: boolean,
  ) => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onFollowThread?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  onReply?: (message: TimelineMessage) => void;
  onSendToChannel?: (message: TimelineMessage) => Promise<void>;
  onUnfollowThread?: (message: TimelineMessage) => void;
  onEntranceComplete?: (messageId: string) => void;
  playEntrance?: boolean;
  projectedThreadRootAuthor?: string | null;
  onOpenProjectedThread?: (message: TimelineMessage) => void;
  profiles?: UserProfileLookup;
  searchQuery?: string;
  showDepthGuides?: boolean;
  videoReviewCommentRootId?: string;
  videoReviewContext?: VideoReviewContext;
};
