import type { AddCommunityPrefillRequest } from "@/features/communities/addCommunityPrefill";
import type { Community } from "@/features/communities/types";
import type { LeaveCommunityResult } from "@/features/communities/leaveCommunity";
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

export type CollapsibleSidebarGroup = "starred" | "channels" | "directMessages";

export type CreateChannelKind = "stream";

export type AppSidebarProps = {
  addCommunityPrefill?: AddCommunityPrefillRequest | null;
  activeCommunity: Community | null;
  channels: Channel[];
  currentPubkey?: string;
  fallbackDisplayName?: string;
  homeBadgeCount: number;
  isPowerActive: boolean;
  isAddCommunityOpen?: boolean;
  workspaceDialogMode: "add-community" | "create" | "join";
  isLoading: boolean;
  isCreatingChannel: boolean;
  profile?: Profile;
  relayConnectionCard: ReturnType<typeof useSidebarRelayConnectionCard>;
  selfPresenceStatus: PresenceStatus;
  showSidebarCollapseButton: boolean;
  errorMessage?: string;
  selectedChannelId: string | null;
  suppressTodaySelection?: boolean;
  selectedView:
    | "today"
    | "home"
    | "channel"
    | "messages"
    | "agents"
    | "clients"
    | "work"
    | "business"
    | "team"
    | "pins";
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  highPriorityUnreadChannelIds: ReadonlySet<string>;
  previewActivityChannelIds: ReadonlySet<string>;
  communities: Community[];
  onAddCommunity: (community: Community) => void;
  onAddCommunityOpenChange?: (open: boolean) => void;
  onCreateChannel: (input: {
    name: string;
    description?: string;
    visibility: ChannelVisibility;
    ttlSeconds?: number;
  }) => Promise<void>;
  onOpenWorkspaceDialog: (mode: "create" | "join") => void;
  onOpenWorkspaceSettings: () => void;
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
  onCreateAgent: () => void;
  onSelectToday: () => void;
  onSelectAgents: () => void;
  onSelectTeam: () => void;
  onSelectPower: () => void;
  onSelectClients?: () => void;
  onSelectWork: () => void;
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
  onRemoveWorkspace: (id: string) => Promise<LeaveCommunityResult | undefined>;
  selfUserStatus?: UserStatus;
  isPresencePending?: boolean;
  onNewMessage: () => void;
  onBackgroundClick?: () => void;
  isCreateChannelOpen?: boolean;
  onCreateChannelOpenChange?: (open: boolean) => void;
  mutedChannelIds?: ReadonlySet<string>;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  starredChannelIds?: ReadonlySet<string>;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
};
