import type { AddCommunityPrefillRequest } from "@/features/communities/addCommunityPrefill";
import type { LeaveCommunityResult } from "@/features/communities/leaveCommunity";
import type { Community } from "@/features/communities/types";
import type { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";
import type { UserStatusInput } from "@/features/user-status/types";
import type {
  Channel,
  ChannelVisibility,
  PresenceStatus,
  Profile,
  SearchHit,
  UserStatus,
} from "@/shared/api/types";

export type CollapsibleSidebarGroup =
  | "starred"
  | "channels"
  | "forums"
  | "directMessages";

export type CreateChannelKind = "stream" | "forum";

export type AppSidebarProps = {
  addCommunityPrefill?: AddCommunityPrefillRequest | null;
  activeCommunity: Community | null;
  channels: Channel[];
  currentPubkey?: string;
  fallbackDisplayName?: string;
  homeBadgeCount: number;
  isAddCommunityOpen?: boolean;
  isLoading: boolean;
  isCreatingChannel: boolean;
  isCreatingForum: boolean;
  profile?: Profile;
  projectsOverviewActive: boolean;
  relayConnectionCard: ReturnType<typeof useSidebarRelayConnectionCard>;
  selfPresenceStatus: PresenceStatus;
  errorMessage?: string;
  selectedChannelId: string | null;
  selectedView:
    | "home"
    | "channel"
    | "messages"
    | "agents"
    | "workflows"
    | "pulse"
    | "projects";
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  previewActivityChannelIds: ReadonlySet<string>;
  communities: Community[];
  onAddCommunity: (community: Community) => void;
  onAddCommunityOpenChange?: (open: boolean) => void;
  onCreateChannel: (input: {
    name: string;
    description?: string;
    visibility: ChannelVisibility;
    ttlSeconds?: number;
    templateId?: string;
  }) => Promise<void>;
  onCreateForum: (input: {
    name: string;
    description?: string;
    visibility: ChannelVisibility;
    ttlSeconds?: number;
    templateId?: string;
  }) => Promise<void>;
  onOpenAddCommunity: () => void;
  onSendFeedback?: () => void;
  onHideDm: (channelId: string) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkAllChannelsRead: () => void;
  onBrowseChannels?: (onCreated?: (channelId: string) => void) => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onUpdateCommunity: (
    id: string,
    updates: Partial<Pick<Community, "name" | "relayUrl" | "token">>,
  ) => void;
  onRemoveCommunity: (id: string) => Promise<LeaveCommunityResult | undefined>;
  onCreateAgent: () => void;
  onSelectAgents: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectWorkflows: () => void;
  onSelectHome: () => void;
  onSelectChannel: (channelId: string) => void;
  onOpenSearchResult: (hit: SearchHit, query: string) => void;
  /** Full channel set for global search, including channels outside the joined sidebar list. */
  searchChannels: Channel[];
  searchFocusRequests: readonly [global: number, channel: number];
  onSelectSettings: (section?: SettingsSection) => void;
  onSetPresenceStatus?: (status: "online" | "away" | "offline") => void;
  onSetUserStatus: (status: UserStatusInput) => void;
  onClearUserStatus: () => void;
  onSwitchCommunity: (id: string) => void;
  selfUserStatus?: UserStatus;
  isPresencePending?: boolean;
  onNewMessage: () => void;
  onBackgroundClick?: () => void;
  isCreateChannelOpen?: boolean;
  isHuddleCompanionOpen?: boolean;
  onHuddleEnded?: (ephemeralChannelId: string | null) => void;
  onCreateChannelOpenChange?: (open: boolean) => void;
  mutedChannelIds?: ReadonlySet<string>;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  starredChannelIds?: ReadonlySet<string>;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
};
