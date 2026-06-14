import { Activity, ArrowUpRight, Brain, Hash } from "lucide-react";

import { MemorySection } from "@/features/agent-memory/ui/MemorySection";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIdentityArchive } from "@/features/identity-archive/hooks";
import type {
  useFollowMutation,
  useUnfollowMutation,
} from "@/features/profile/hooks";
import {
  ProfileIngressRow,
  ProfilePrimaryActions,
  ProfileWorkingBadge,
} from "@/features/profile/ui/ProfileActions";
import {
  buildOwnerFields,
  buildPublicFields,
  ProfileFieldGroup,
} from "@/features/profile/ui/ProfileFields";
import { ProfileHero } from "@/features/profile/ui/ProfileHero";
import { ProfileManageSection } from "@/features/profile/ui/ProfileManageSection";
import type {
  ProfileSummaryData,
  ProfileSummaryUserStatus,
} from "@/features/profile/ui/profileSummaryTypes";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";

// ── Summary view ─────────────────────────────────────────────────────────────

export type ProfileSummaryViewProps = {
  canEditAgent: boolean;
  canViewActivity: boolean;
  channelCount: number;
  channelIdToName: Record<string, string>;
  channelsLoading: boolean;
  displayName: string;
  followMutation: ReturnType<typeof useFollowMutation>;
  handleEditAgent: () => void;
  handleMessage: () => void;
  handleOpenActivity: () => void;
  isBot: boolean;
  isFollowing: boolean;
  isOwner: boolean | undefined;
  isSelf: boolean;
  managedAgent: ManagedAgent | undefined;
  memoriesLoading: boolean;
  memoryCount: number | undefined;
  ownerDisplayName: string | null;
  ownerHandle: string | null;
  onOpenChannels: () => void;
  onOpenMemories: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  presenceLoaded: boolean;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ProfileSummaryData;
  pubkey: string;
  relayAgent: RelayAgent | undefined;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
  userStatus: ProfileSummaryUserStatus;
};

export function ProfileSummaryView({
  canEditAgent,
  canViewActivity,
  channelCount,
  channelIdToName,
  channelsLoading,
  displayName,
  followMutation,
  handleEditAgent,
  handleMessage,
  handleOpenActivity,
  isBot,
  isFollowing,
  isOwner,
  isSelf,
  managedAgent,
  memoriesLoading,
  memoryCount,
  ownerDisplayName,
  ownerHandle,
  onOpenChannels,
  onOpenMemories,
  onOpenDm,
  presenceLoaded,
  presenceStatus,
  profile,
  pubkey,
  relayAgent,
  unfollowMutation,
  userStatus,
}: ProfileSummaryViewProps) {
  const { goChannel } = useAppNavigation();
  const activeTurns = useActiveAgentTurns(isBot ? pubkey : null);

  const { canArchive, isArchived } = useIdentityArchive(pubkey);

  const metadataFields = [
    ...buildPublicFields({
      pubkey,
      profile,
      relayAgent,
      isBot,
    }),
    ...(isOwner === true
      ? buildOwnerFields({
          managedAgent,
          ownerDisplayName,
          ownerHandle,
          presenceLoaded,
          presenceStatus,
          relayAgent,
        })
      : []),
  ];

  const showMemoriesIngress = isOwner === true;
  const showChannelsIngress =
    channelsLoading || channelCount > 0 || isBot || relayAgent !== undefined;

  const showManageSection = canArchive && isArchived !== undefined;

  return (
    <div className="flex flex-col gap-6 pt-4">
      <ProfileHero
        displayName={displayName}
        isArchived={isArchived}
        isBot={isBot}
        presenceStatus={presenceStatus}
        profile={profile}
        userStatus={userStatus}
      />

      {!isSelf ? (
        <ProfilePrimaryActions
          canEditAgent={canEditAgent}
          followMutation={followMutation}
          onEditAgent={handleEditAgent}
          isFollowing={isFollowing}
          onMessage={onOpenDm ? handleMessage : undefined}
          pubkey={pubkey}
          unfollowMutation={unfollowMutation}
        />
      ) : null}

      {activeTurns.length > 0 ? (
        <div className="flex flex-wrap justify-center gap-1.5">
          {activeTurns.map(({ channelId, observedAt }) => (
            <ProfileWorkingBadge
              key={channelId}
              channelId={channelId}
              name={channelIdToName[channelId] ?? channelId}
              observedAt={observedAt}
              onNavigate={goChannel}
            />
          ))}
        </div>
      ) : null}

      {showMemoriesIngress || showChannelsIngress || canViewActivity ? (
        <section className="space-y-2">
          {showMemoriesIngress ? (
            <ProfileIngressRow
              icon={Brain}
              label="Memories"
              onClick={onOpenMemories}
              testId="user-profile-memories-ingress"
              trailing={
                memoriesLoading
                  ? "Loading…"
                  : memoryCount !== undefined
                    ? String(memoryCount)
                    : "View"
              }
            />
          ) : null}
          {showChannelsIngress ? (
            <ProfileIngressRow
              icon={Hash}
              label="Channels"
              onClick={onOpenChannels}
              testId="user-profile-channels-ingress"
              trailing={
                channelsLoading
                  ? "Loading…"
                  : channelCount > 0
                    ? String(channelCount)
                    : "None"
              }
            />
          ) : null}
          {canViewActivity ? (
            <ProfileIngressRow
              icon={Activity}
              label="Activity log"
              onClick={handleOpenActivity}
              testId={`user-profile-view-activity-${pubkey}`}
            />
          ) : null}
        </section>
      ) : null}

      {metadataFields.length > 0 ? (
        <ProfileFieldGroup fields={metadataFields} />
      ) : null}

      {showManageSection ? (
        <ProfileManageSection isBot={isBot} pubkey={pubkey} />
      ) : null}
    </div>
  );
}

// ── Focused views ────────────────────────────────────────────────────────────

export function MemoryFocusedView({
  agentPubkey,
  isOwner,
}: {
  agentPubkey: string;
  isOwner: boolean | undefined;
}) {
  if (isOwner !== true) {
    return null;
  }

  return (
    <div className="pt-4">
      <MemorySection agentPubkey={agentPubkey} />
    </div>
  );
}

type ProfileChannelLink = {
  id: string;
  name: string;
};

export function ChannelsFocusedView({
  channels,
  isLoading,
  onOpenChannel,
}: {
  channels: ProfileChannelLink[];
  isLoading: boolean;
  onOpenChannel: (channelId: string) => void;
}) {
  if (isLoading) {
    return (
      <p className="pt-4 text-base leading-7 text-muted-foreground">
        Loading channels…
      </p>
    );
  }

  if (channels.length === 0) {
    return (
      <p
        className="pt-4 text-base leading-7 italic text-muted-foreground"
        data-testid="user-profile-channels-empty"
      >
        No visible channel memberships.
      </p>
    );
  }

  return (
    <ul
      className="overflow-hidden rounded-2xl bg-muted/20"
      data-testid="user-profile-channels-list"
    >
      {channels.map((channel) => (
        <li key={channel.id}>
          <button
            aria-label={`Open #${channel.name}`}
            className="group flex w-full items-center gap-3 px-4 py-3 text-left text-base leading-7 text-foreground transition-colors hover:bg-muted/40"
            data-testid={`user-profile-channel-link-${channel.name}`}
            onClick={() => onOpenChannel(channel.id)}
            type="button"
          >
            <span className="min-w-0 flex-1 truncate">#{channel.name}</span>
            <ArrowUpRight
              aria-hidden="true"
              className="h-4 w-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground"
            />
          </button>
        </li>
      ))}
    </ul>
  );
}
