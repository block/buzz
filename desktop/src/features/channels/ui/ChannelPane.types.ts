import type * as React from "react";
import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import type { ChannelAgentSessionAgent } from "@/features/channels/ui/useChannelAgentSessions";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import type { TimelineMessage } from "@/features/messages/types";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { useChannelFind } from "@/features/search/useChannelFind";
import type {
  ProfilePanelTab,
  ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanel";
import type { Channel } from "@/shared/api/types";
export type ChannelPaneProps = {
  activeChannel: Channel | null;
  activityAgents?: BotActivityAgent[];
  agentPubkeys?: ReadonlySet<string>;
  agentPubkeysPending?: boolean;
  agentSessionAgents: ChannelAgentSessionAgent[];
  botTypingEntries: TypingIndicatorEntry[];
  channelFind: ReturnType<typeof useChannelFind>;
  channelManagementOpen?: boolean;
  currentPubkey?: string;
  editTarget?: {
    author: string;
    body: string;
    id: string;
    imetaMedia?: ImetaMedia[];
  } | null;
  fetchOlder?: () => Promise<void>;
  header?: React.ReactNode;
  hasOlderMessages?: boolean;
  isFetchingOlder?: boolean;
  isJoining?: boolean;
  isSinglePanelView?: boolean;
  isSending: boolean;
  isTimelineLoading: boolean;
  messages: TimelineMessage[];
  threadSummaries?: ReadonlyMap<string, ChannelWindowThreadSummary>;
  firstUnreadMessageId?: string | null;
  unreadCount?: number;
  canResetThreadPanelWidth: boolean;
  onCancelEdit?: () => void;
  onCancelThreadReply: () => void;
  /**
   * Fired by the header back arrow when Activity has a captured pane to
   * return to. Absent (arrow hidden) for composer/no-pane opens and
   * direct/restored Activity URLs — the close affordance is the fallback.
   */
  onBackFromAgentSession?: () => void;
  onCloseAgentSession: () => void;
  onCloseChannelManagement?: () => void;
  onChannelManagementDeleted?: () => void;
  onCloseProfilePanel: () => void;
  onAddAgent?: () => void;
  onCreateChannel?: () => void;
  onCloseThread: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onExpandThreadReplies: (message: TimelineMessage) => void;
  onJoinChannel?: () => Promise<void>;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  onOpenDm?: (pubkeys: string[]) => Promise<void> | void;
  onOpenMembers?: () => void;
  onOpenProfilePanel: (pubkey: string) => void;
  onOpenThread: (message: TimelineMessage) => void;
  onResetThreadPanelWidth: () => void;
  onSelectThreadReplyTarget: (message: TimelineMessage) => void;
  onSendMessage: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
  ) => Promise<void>;
  onSendVideoReviewComment?: (
    message: TimelineMessage,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    parentEventId?: string,
  ) => Promise<void>;
  onSendThreadReply: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
    threadContext?: {
      parentEventId: string | null;
      threadHeadId: string | null;
    } | null,
  ) => Promise<void>;
  onTargetReached?: (messageId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  onThreadScrollTargetResolved: () => void;
  onThreadPanelResizeStart: (
    event: React.PointerEvent<HTMLButtonElement>,
  ) => void;
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  openThreadHeadId: string | null;
  shouldShowThreadSkeleton: boolean;
  openAgentSessionChannelId: string | null;
  openAgentSessionPubkey: string | null;
  onProfilePanelViewChange: (
    view: ProfilePanelView,
    options?: { replace?: boolean },
  ) => void;
  onProfilePanelTabChange: (
    tab: ProfilePanelTab,
    options?: { replace?: boolean },
  ) => void;
  profilePanelPubkey?: string | null;
  profilePanelTab: ProfilePanelTab;
  profilePanelView: ProfilePanelView;
  threadHeadMessage: TimelineMessage | null;
  threadMessages: MainTimelineEntry[];
  threadPanelWidthPx: number;
  threadTypingPubkeys: string[];
  threadReplyTargetMessage: TimelineMessage | null;
  threadScrollTargetId: string | null;
  threadUnreadCounts?: ReadonlyMap<string, number>;
  threadReplyUnreadCounts?: ReadonlyMap<string, number>;
  threadFirstUnreadReplyId?: string | null;
  targetMessageId: string | null;
  typingPubkeys: string[];
  isFollowingThread?: boolean;
  onFollowThread?: () => void;
  onUnfollowThread?: () => void;
  followThreadById?: (rootId: string) => void;
  unfollowThreadById?: (rootId: string) => void;
  isFollowingThreadById?: (rootId: string) => boolean;
  isMessageUnreadById?: (messageId: string) => boolean;
};
