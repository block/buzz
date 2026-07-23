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
  /**
   * When non-null, the main composer fires `submitMessage` once after loading
   * the draft identified by this key — i.e. the user clicked "Send message"
   * in the Drafts panel and confirmed. Cleared by the composer after firing so
   * back-navigation cannot re-trigger.
   */
  autoSendDraftKey?: string | null;
  /**
   * Called after the auto-submit guard fires to surgically clear `?autoSend`
   * from the URL while preserving `?thread` and all other panel search state.
   * If omitted, ChannelPane falls back to a full goChannel() re-navigation
   * (safe for the main-composer path, which carries no URL-backed thread).
   */
  onAutoSendComplete?: (() => void) | null;
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
  /** True when the loaded window provably starts at the channel's beginning. */
  historyExhausted?: boolean;
  isFetchingOlder?: boolean;
  isJoining?: boolean;
  isSinglePanelView?: boolean;
  isSending: boolean;
  isTimelineLoading: boolean;
  /** Newly-created message that should receive the one-shot conversation arrival motion. */
  entranceMessageId?: string | null;
  onEntranceMessageComplete?: (messageId: string) => void;
  /** Welcome kickoff characters, rendered standing on the Welcome composer banner. */
  welcomeKickoffStage?: React.ReactNode;
  /** The kickoff is still setting up the team — the banner copy reads as setup status. */
  welcomeKickoffSettingUp?: boolean;
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
  onAddAgent?: (options?: { beforeSend?: () => void }) => void;
  onBrowseChannels?: () => void;
  onCreateChannel?: () => void;
  onCloseThread: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditSave?: (
    content: string,
    mediaTags?: string[][],
    mentionPubkeys?: string[],
  ) => Promise<void>;
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
  ownerProfiles?: UserProfileLookup;
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
  threadMessagesPending?: boolean;
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
