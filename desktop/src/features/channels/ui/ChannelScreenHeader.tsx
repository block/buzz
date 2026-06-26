import { LogIn } from "lucide-react";
import * as React from "react";

import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import type { EphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import type { ActiveDmHeaderParticipant } from "@/features/channels/useActiveChannelHeader";
import { getChannelDescription } from "@/features/channels/lib/channelDescription";
import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import { ChannelHeaderStatusBadge } from "@/features/channels/ui/ChannelHeaderStatusBadge";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { Button } from "@/shared/ui/button";
import type { Channel, PresenceStatus } from "@/shared/api/types";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type ChannelSurfaceTab = "messages" | "tasks";

type ChannelScreenHeaderProps = {
  activeChannel: Channel | null;
  activeChannelEphemeralDisplay: EphemeralChannelDisplay | null;
  activeChannelTitle: string;
  activeSurfaceTab?: ChannelSurfaceTab;
  actionsVariant?: "inline" | "compact";
  activeDmAvatarUrl: string | null;
  activeDmHeaderParticipants: ActiveDmHeaderParticipant[];
  activeDmPresenceStatus: PresenceStatus | null;
  chromeWrapperRef?: React.Ref<HTMLDivElement>;
  currentPubkey?: string;
  isAddBotOpen?: boolean;
  isJoining?: boolean;
  showHeaderContent?: boolean;
  onAddBotOpenChange?: (open: boolean) => void;
  onJoinChannel?: () => Promise<void>;
  onManageChannel: () => void;
  onSurfaceTabChange?: (tab: ChannelSurfaceTab) => void;
  onToggleMembers: () => void;
};

const CHANNEL_SURFACE_TAB_LIST_CLASS =
  "relative h-auto w-full justify-start gap-6 rounded-none bg-transparent p-0 text-muted-foreground";
const CHANNEL_SURFACE_TAB_TRIGGER_CLASS =
  "relative z-10 rounded-none border-0 bg-transparent px-0 py-2 text-sm font-medium shadow-none transition-colors duration-150 ease-out data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none";

export function ChannelScreenHeader({
  activeChannel,
  activeChannelEphemeralDisplay,
  activeChannelTitle,
  activeSurfaceTab = "messages",
  actionsVariant = "inline",
  activeDmAvatarUrl,
  activeDmHeaderParticipants,
  activeDmPresenceStatus,
  chromeWrapperRef,
  currentPubkey,
  isAddBotOpen,
  isJoining = false,
  onAddBotOpenChange,
  showHeaderContent = true,
  onJoinChannel,
  onManageChannel,
  onSurfaceTabChange,
  onToggleMembers,
}: ChannelScreenHeaderProps) {
  const isGroupDm =
    activeChannel?.channelType === "dm" &&
    activeDmHeaderParticipants.length > 1;
  const showJoinButton =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt &&
    onJoinChannel;
  const showSurfaceTabs =
    activeChannel?.channelType === "stream" && Boolean(onSurfaceTabChange);
  const tabListRef = React.useRef<HTMLDivElement>(null);
  const tabTriggerRefs = React.useRef<
    Record<ChannelSurfaceTab, HTMLButtonElement | null>
  >({
    messages: null,
    tasks: null,
  });
  const [tabIndicator, setTabIndicator] = React.useState({
    left: 0,
    width: 0,
  });

  const updateTabIndicator = React.useCallback(() => {
    const list = tabListRef.current;
    const trigger = tabTriggerRefs.current[activeSurfaceTab];

    if (!showSurfaceTabs || !list || !trigger) {
      return;
    }

    const nextIndicator = {
      left: trigger.offsetLeft,
      width: trigger.offsetWidth,
    };

    setTabIndicator((current) =>
      Math.abs(current.left - nextIndicator.left) < 0.5 &&
      Math.abs(current.width - nextIndicator.width) < 0.5
        ? current
        : nextIndicator,
    );
  }, [activeSurfaceTab, showSurfaceTabs]);

  React.useLayoutEffect(() => {
    updateTabIndicator();

    if (!showSurfaceTabs) {
      return;
    }

    let isCancelled = false;
    const updateIfActive = () => {
      if (!isCancelled) {
        updateTabIndicator();
      }
    };
    const frameId = window.requestAnimationFrame(updateIfActive);
    const observer = new ResizeObserver(updateTabIndicator);
    const list = tabListRef.current;

    void document.fonts.ready.then(updateIfActive);

    if (list) {
      observer.observe(list);
    }

    for (const trigger of Object.values(tabTriggerRefs.current)) {
      if (trigger) {
        observer.observe(trigger);
      }
    }

    return () => {
      isCancelled = true;
      window.cancelAnimationFrame(frameId);
      observer.disconnect();
    };
  }, [showSurfaceTabs, updateTabIndicator]);

  const actions = activeChannel ? (
    showJoinButton ? (
      <Button
        disabled={isJoining}
        onClick={() => void onJoinChannel()}
        size="sm"
        variant="default"
      >
        <LogIn className="mr-1.5 h-4 w-4" />
        {isJoining ? "Joining…" : "Join"}
      </Button>
    ) : (
      <ChannelMembersBar
        channel={activeChannel}
        currentPubkey={currentPubkey}
        isAddBotOpen={isAddBotOpen}
        onAddBotOpenChange={onAddBotOpenChange}
        onManageChannel={onManageChannel}
        onToggleMembers={onToggleMembers}
        variant={actionsVariant}
      />
    )
  ) : null;

  if (!showHeaderContent) {
    return null;
  }

  const surfaceTabs = showSurfaceTabs ? (
    <Tabs
      className="shrink-0"
      onValueChange={(value) =>
        onSurfaceTabChange?.(value as ChannelSurfaceTab)
      }
      value={activeSurfaceTab}
    >
      <TabsList className={CHANNEL_SURFACE_TAB_LIST_CLASS} ref={tabListRef}>
        <span
          aria-hidden="true"
          className="pointer-events-none absolute bottom-0 left-0 z-20 h-[3px] origin-left rounded-[3px] bg-foreground opacity-0 transition-[transform,width,opacity] duration-[180ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none data-[ready=true]:opacity-100"
          data-ready={tabIndicator.width > 0}
          data-testid="channel-surface-tab-indicator"
          style={{
            transform: `translate3d(${tabIndicator.left}px, 0, 0)`,
            width: tabIndicator.width,
          }}
        />
        <TabsTrigger
          className={CHANNEL_SURFACE_TAB_TRIGGER_CLASS}
          ref={(element) => {
            tabTriggerRefs.current.messages = element;
          }}
          value="messages"
        >
          Messages
        </TabsTrigger>
        <TabsTrigger
          className={CHANNEL_SURFACE_TAB_TRIGGER_CLASS}
          ref={(element) => {
            tabTriggerRefs.current.tasks = element;
          }}
          value="tasks"
        >
          Tasks
        </TabsTrigger>
      </TabsList>
    </Tabs>
  ) : undefined;

  return (
    <ChatHeader
      belowTitleContent={surfaceTabs}
      belowTitleContentClassName="mt-5"
      belowSystemChrome
      chromeWrapperRef={chromeWrapperRef}
      actions={actions}
      channelType={activeChannel?.channelType}
      description={getChannelDescription(activeChannel)}
      leadingContent={
        activeChannel?.channelType === "dm" ? (
          isGroupDm ? (
            <DmHeaderParticipantStack
              participants={activeDmHeaderParticipants}
            />
          ) : (
            <ProfileAvatar
              avatarUrl={activeDmAvatarUrl}
              className="h-6 w-6 rounded-full text-2xs"
              iconClassName="h-3.5 w-3.5"
              label={activeChannelTitle}
              testId="chat-header-dm-avatar"
            />
          )
        ) : undefined
      }
      statusBadge={
        <ChannelHeaderStatusBadge
          channelType={activeChannel?.channelType}
          ephemeralDisplay={activeChannelEphemeralDisplay}
          presenceStatus={isGroupDm ? null : activeDmPresenceStatus}
        />
      }
      title={activeChannelTitle}
      visibility={activeChannel?.visibility}
    />
  );
}

function DmHeaderParticipantStack({
  participants,
}: {
  participants: ActiveDmHeaderParticipant[];
}) {
  const { hiddenCount, visibleParticipants } =
    getDmParticipantPreview(participants);
  const stackItemCount = visibleParticipants.length + (hiddenCount > 0 ? 1 : 0);

  return (
    <div
      aria-hidden="true"
      className="mr-1 flex shrink-0 items-center"
      data-testid="chat-header-dm-avatar-stack"
    >
      {visibleParticipants.map((participant, index) => (
        <div
          className={index > 0 ? "-ml-2" : ""}
          data-testid="chat-header-dm-avatar-stack-participant"
          key={participant.pubkey}
          style={{
            zIndex: index + 1,
            ...(index < stackItemCount - 1 && {
              mask: "radial-gradient(circle 16px at calc(100% + 4px) 50%, transparent 99%, #fff 100%)",
              WebkitMask:
                "radial-gradient(circle 16px at calc(100% + 4px) 50%, transparent 99%, #fff 100%)",
            }),
          }}
        >
          <UserAvatar
            avatarUrl={participant.avatarUrl}
            className="h-7 w-7 text-2xs"
            displayName={participant.displayName}
            size="sm"
          />
        </div>
      ))}
      {hiddenCount > 0 ? (
        <div
          className={visibleParticipants.length > 0 ? "-ml-2" : ""}
          data-testid="chat-header-dm-avatar-stack-more"
          style={{ zIndex: stackItemCount }}
        >
          <span className="flex h-7 w-7 items-center justify-center rounded-full bg-secondary font-semibold text-secondary-foreground shadow-xs">
            <span className="text-2xs leading-none">+{hiddenCount}</span>
          </span>
        </div>
      ) : null}
    </div>
  );
}
