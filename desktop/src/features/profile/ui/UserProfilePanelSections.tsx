import * as React from "react";
import { ChevronDown, ChevronUp, Pencil } from "lucide-react";

import { OtherSetupAgentMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { useIsOtherSetupAgent } from "@/features/agents/useKnownAgentPubkeys";
import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import { agentPresenceStartBlockReason } from "@/features/agents/lib/useAgentAvailability";
import {
  getManagedAgentPrimaryActionLabel,
  isManagedAgentActive,
} from "@/features/agents/lib/managedAgentControlActions";
import { RestartDiffBadge } from "@/features/agents/ui/RestartDiffBadge";
import { AgentConfigPanel } from "@/features/agents/ui/AgentConfigPanel";
import type { IdentityArchiveActions } from "@/features/identity-archive/hooks";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type { ProfileActivityAgent } from "@/features/profile/lib/profileActivityAgent";
import type {
  useFollowMutation,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import type { ProfileField } from "@/features/profile/ui/UserProfilePanelFields";
import { AGENT_DETAILS_FIELD_LABELS } from "@/features/profile/ui/UserProfilePanelAgentDetails";
import {
  ProfileInfoTabContent,
  ProfileRuntimeTabContent,
  ProfileTabBar,
} from "@/features/profile/ui/UserProfilePanelTabs";
import {
  MaskedAvatarBadgeFrame,
  STATUS_DOT_MASK_CURVE,
} from "@/features/profile/ui/MaskedAvatarBadgeFrame";
import { ProfileTabContentTransition } from "@/features/profile/ui/ProfileTabContentTransition";
import {
  ChannelsFocusedView,
  MemoryFocusedView,
} from "@/features/profile/ui/UserProfilePanelFocusedViews";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  ProfilePersonaPrimaryActions,
  ProfilePrimaryActions,
} from "@/features/profile/ui/UserProfilePrimaryActions";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import type {
  ProfileChannelLink,
  ProfilePanelTab,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { cn } from "@/shared/lib/cn";
import { observeElementBlockSize } from "@/shared/layout/observeElementBlockSize";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { Badge } from "@/shared/ui/badge";

export { AgentInstructionsFocusedView } from "@/features/profile/ui/UserProfilePanelAgentDetails";

// ── Summary view ─────────────────────────────────────────────────────────────

export type ProfileSummaryViewProps = {
  activityAgent: ProfileActivityAgent | null;
  canAddToChannel: boolean;
  canDeleteAgent: boolean;
  canEditAgent: boolean;
  canOpenAgentLogs: boolean;
  canViewActivity: boolean;
  channelCount: number;
  channelIdToName: Record<string, string>;
  channels: ProfileChannelLink[];
  channelsLoading: boolean;
  displayName: string;
  followMutation: ReturnType<typeof useFollowMutation>;
  canInstantiateAgent: boolean;
  agentInstruction: string | null;
  handleAgentPrimaryAction: () => void;
  handleAgentRestart: () => void;
  handleEditAgent: () => void;
  handleToggleAgentAutoStart: () => void;
  handleEditPersona?: () => void;
  handleHuddle?: () => void;
  handleInstantiateAgent: () => void;
  handleMessage?: () => void;
  handleWave?: () => void;
  isArchived: boolean;
  isHuddlePending: boolean;
  isMessagePending: boolean;
  isWavePending: boolean;
  isBot: boolean;
  isAgentActionPending: boolean;
  isFollowing: boolean;
  isOwner: boolean | undefined;
  isSelf: boolean;
  instanceBuckets: { live: ManagedAgent[]; archived: ManagedAgent[] };
  managedAgent: ManagedAgent | undefined;
  agentInfoFields: ProfileField[];
  archiveActions: IdentityArchiveActions;
  agentSettingsFields: ProfileField[];
  diagnosticsFields: ProfileField[];
  onAddToChannel: () => void;
  /** Mint an agent trading card. Present only for owner-managed personas. */
  onCreateCard?: () => void;
  onDeleteAgent: () => void;
  onDuplicateAgent?: () => void;
  onExportAgent?: () => void;
  onOpenInstance: (pubkey: string) => void;
  onOpenActivity: (channelId?: string | null) => void;
  onOpenChannel: (channelId: string) => void;
  onOpenDiagnostics: () => void;
  onStickyChromeChange: (state: { active: boolean; height: number }) => void;
  onTabChange: (tab: ProfilePanelTab, options?: { replace?: boolean }) => void;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ReturnType<typeof useUserProfileQuery>["data"];
  pubkey: string | null;
  relayAgent: RelayAgent | undefined;
  tab: ProfilePanelTab;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
  userStatus: { text: string; emoji: string } | null | undefined;
};

const PROFILE_HERO_SPACING = {
  "0": 0,
  "6": 24,
} as const;
const PROFILE_ACTIONS_SCROLL_DISTANCE_PX = 96;

const PROFILE_HERO_PRESENCE_BADGE = {
  cutout: { cx: 68, cy: 68, r: 15 },
  shell: {
    bottom: PROFILE_HERO_SPACING["0"],
    height: PROFILE_HERO_SPACING["6"],
    right: PROFILE_HERO_SPACING["0"],
    width: PROFILE_HERO_SPACING["6"],
  },
} as const;

export function ProfileSummaryView({
  activityAgent,
  canAddToChannel,
  canDeleteAgent,
  canEditAgent,
  canOpenAgentLogs,
  canViewActivity,
  channelCount,
  channelIdToName,
  channels,
  channelsLoading,
  displayName,
  followMutation,
  canInstantiateAgent,
  agentInstruction,
  handleAgentPrimaryAction,
  handleAgentRestart,
  handleEditAgent,
  handleToggleAgentAutoStart,
  handleEditPersona,
  handleHuddle,
  handleInstantiateAgent,
  handleMessage,
  handleWave,
  isArchived,
  isHuddlePending,
  isMessagePending,
  isWavePending,
  isBot,
  isAgentActionPending,
  isFollowing,
  isOwner,
  isSelf,
  instanceBuckets,
  managedAgent,
  agentInfoFields,
  archiveActions,
  agentSettingsFields,
  diagnosticsFields,
  onAddToChannel,
  onCreateCard,
  onDeleteAgent,
  onDuplicateAgent,
  onExportAgent,
  onOpenInstance,
  onOpenActivity,
  onOpenChannel,
  onOpenDiagnostics,
  onStickyChromeChange,
  onTabChange,
  presenceStatus,
  profile,
  pubkey,
  relayAgent,
  tab,
  unfollowMutation,
  userStatus,
}: ProfileSummaryViewProps) {
  const notManagedOnDevice = useIsOtherSetupAgent(pubkey, profile?.ownerPubkey);
  const activeTurns = useAgentWorking(isBot ? pubkey : null).channels;
  const stickyLayoutRef = React.useRef<HTMLDivElement>(null);
  const [primaryActionsConcealed, setPrimaryActionsConcealed] =
    React.useState(false);
  const [primaryActionsElement, setPrimaryActionsElement] =
    React.useState<HTMLDivElement | null>(null);
  const [stickyTabsElement, setStickyTabsElement] =
    React.useState<HTMLDivElement | null>(null);
  const stickyHeroRef = useMeasuredCssVariable({
    cssVariable: "--buzz-profile-sticky-hero-height",
    enabled: isBot,
    resetKey: displayName,
    resetValue: "0px",
    targetRef: stickyLayoutRef,
  });

  const showMemoriesTab = isOwner === true && Boolean(pubkey);
  const showInstructionBlock =
    isOwner === true &&
    (agentInstruction !== null || handleEditPersona !== undefined);
  const showChannelsTab =
    channelsLoading || channelCount > 0 || isBot || relayAgent !== undefined;
  const runtimeConfigurationFields = agentSettingsFields.filter((field) =>
    AGENT_DETAILS_FIELD_LABELS.has(field.label),
  );
  const runtimeSettingsFields = agentSettingsFields.filter(
    (field) => !AGENT_DETAILS_FIELD_LABELS.has(field.label),
  );
  const runtimeFields = [
    ...runtimeConfigurationFields,
    ...runtimeSettingsFields,
  ];
  const showRuntimeTab =
    isOwner === true &&
    isBot &&
    (managedAgent !== undefined ||
      runtimeConfigurationFields.length > 0 ||
      runtimeSettingsFields.length > 0 ||
      instanceBuckets.live.length > 0 ||
      instanceBuckets.archived.length > 0 ||
      diagnosticsFields.length > 0 ||
      canOpenAgentLogs);
  const showDiagnosticsIngress =
    diagnosticsFields.some((field) => field.label !== "Status") ||
    canOpenAgentLogs;
  const showActivityIngress = canViewActivity;
  const showInfoTab =
    agentInfoFields.length > 0 ||
    runtimeFields.length > 0 ||
    isArchived ||
    showActivityIngress ||
    showInstructionBlock ||
    managedAgent !== undefined ||
    !showRuntimeTab;

  const diagnosticsErrorField = diagnosticsFields.find(
    (field) => field.label === "Last error",
  );
  const diagnosticsTrailing =
    diagnosticsErrorField !== undefined ? (
      <Badge
        className="normal-case tracking-normal"
        title={diagnosticsErrorField.displayValue}
        variant="destructive"
      >
        Error
      </Badge>
    ) : null;
  const tabs = React.useMemo(() => {
    const items: Array<{
      id: ProfilePanelTab;
      label: string;
      trailing?: React.ReactNode;
    }> = [];
    if (showInfoTab) {
      items.push({ id: "info", label: "Info" });
    }
    if (showRuntimeTab) {
      items.push({ id: "runtime", label: "Runtime" });
    }
    if (showChannelsTab) {
      items.push({ id: "channels", label: "Channels" });
    }
    if (showMemoriesTab) {
      items.push({
        id: "memories",
        label: "Memories",
      });
    }
    return items;
  }, [showChannelsTab, showInfoTab, showMemoriesTab, showRuntimeTab]);

  const showTabSection = tabs.length > 0;
  const showTabBar = !(tabs.length === 1 && tabs[0]?.id === "info");
  const activeTab = tabs.some((item) => item.id === tab)
    ? tab
    : (tabs[0]?.id ?? "info");
  const tabIds = React.useMemo(() => tabs.map((item) => item.id), [tabs]);

  React.useLayoutEffect(() => {
    const layout = stickyLayoutRef.current;
    const scrollBody = layout?.closest<HTMLElement>(
      '[data-testid="user-profile-scroll-body"]',
    );
    const panel = scrollBody?.parentElement;
    const header = panel?.querySelector<HTMLElement>(
      '[data-testid="user-profile-panel-header"]',
    );
    const hero = layout?.querySelector<HTMLElement>(
      '[data-testid="user-profile-sticky-hero"]',
    );

    if (
      !isBot ||
      !showTabBar ||
      !scrollBody ||
      !header ||
      !hero ||
      !stickyTabsElement
    ) {
      if (primaryActionsElement) {
        primaryActionsElement.style.opacity = "";
      }
      setPrimaryActionsConcealed(false);
      onStickyChromeChange({ active: false, height: 0 });
      return;
    }

    let animationFrame: number | null = null;
    const updateScrollChrome = () => {
      animationFrame = null;
      const headerRect = header.getBoundingClientRect();
      const heroRect = hero.getBoundingClientRect();
      const tabsRect = stickyTabsElement.getBoundingClientRect();

      if (primaryActionsElement) {
        const actionsProgress = Math.min(
          Math.max(
            scrollBody.scrollTop / PROFILE_ACTIONS_SCROLL_DISTANCE_PX,
            0,
          ),
          1,
        );
        primaryActionsElement.style.opacity = String(1 - actionsProgress);
        setPrimaryActionsConcealed(actionsProgress >= 1);
      }
      onStickyChromeChange({
        active: scrollBody.scrollTop > 0 && tabsRect.top <= heroRect.bottom + 1,
        height: Math.max(0, Math.ceil(tabsRect.bottom - headerRect.top)),
      });
    };
    const scheduleStickyChromeUpdate = () => {
      if (animationFrame === null) {
        animationFrame = window.requestAnimationFrame(updateScrollChrome);
      }
    };

    const disconnectObservers = [header, hero, stickyTabsElement].map(
      (element) => observeElementBlockSize(element, scheduleStickyChromeUpdate),
    );
    scrollBody.addEventListener("scroll", scheduleStickyChromeUpdate, {
      passive: true,
    });
    window.addEventListener("resize", scheduleStickyChromeUpdate);
    scheduleStickyChromeUpdate();

    return () => {
      if (animationFrame !== null) {
        window.cancelAnimationFrame(animationFrame);
      }
      for (const disconnect of disconnectObservers) {
        disconnect();
      }
      scrollBody.removeEventListener("scroll", scheduleStickyChromeUpdate);
      window.removeEventListener("resize", scheduleStickyChromeUpdate);
      if (primaryActionsElement) {
        primaryActionsElement.style.opacity = "";
      }
      setPrimaryActionsConcealed(false);
      onStickyChromeChange({ active: false, height: 0 });
    };
  }, [
    isBot,
    onStickyChromeChange,
    primaryActionsElement,
    showTabBar,
    stickyTabsElement,
  ]);

  const primaryActionsMotionClassName = isBot
    ? cn(
        "will-change-[opacity]",
        primaryActionsConcealed && "pointer-events-none",
      )
    : undefined;

  return (
    <div
      className="flex flex-col gap-6 pt-4"
      data-testid="user-profile-summary-scroll-layout"
      ref={stickyLayoutRef}
    >
      <div
        className={cn(isBot && "sticky top-0 z-40 -mx-4 px-4")}
        data-testid={isBot ? "user-profile-sticky-hero" : undefined}
        ref={stickyHeroRef}
      >
        <ProfileHero
          displayName={displayName}
          notManagedOnDevice={notManagedOnDevice}
          isBot={isBot}
          onEditAgent={canEditAgent ? handleEditAgent : undefined}
          presenceStatus={presenceStatus}
          profile={profile}
          userStatus={userStatus}
        />
      </div>

      {canInstantiateAgent ? (
        <ProfilePersonaPrimaryActions
          actionGroupRef={setPrimaryActionsElement}
          className={primaryActionsMotionClassName}
          concealed={primaryActionsConcealed}
          disabled={isAgentActionPending}
          onStartAgent={handleInstantiateAgent}
        />
      ) : !isSelf && pubkey ? (
        <ProfilePrimaryActions
          actionGroupRef={setPrimaryActionsElement}
          className={primaryActionsMotionClassName}
          concealed={primaryActionsConcealed}
          followMutation={followMutation}
          agentActionDisabled={isAgentActionPending}
          agentStartBlockReason={
            managedAgent
              ? agentPresenceStartBlockReason(
                  isManagedAgentActive(managedAgent),
                  presenceStatus,
                )
              : undefined
          }
          agentActionLabel={
            isOwner === true && managedAgent
              ? getManagedAgentPrimaryActionLabel(managedAgent)
              : undefined
          }
          agentActionLive={
            managedAgent?.status === "running" ||
            managedAgent?.status === "deployed"
          }
          onAgentPrimaryAction={
            isOwner === true && managedAgent
              ? handleAgentPrimaryAction
              : undefined
          }
          onAgentRestart={
            isOwner === true &&
            managedAgent?.backend.type === "local" &&
            (managedAgent.status === "running" ||
              managedAgent.status === "deployed")
              ? handleAgentRestart
              : undefined
          }
          isFollowing={isFollowing}
          huddlePending={isBot ? undefined : isHuddlePending}
          messagePending={isMessagePending}
          onHuddle={isBot ? undefined : handleHuddle}
          onMessage={handleMessage}
          onWave={handleWave}
          pubkey={pubkey}
          unfollowMutation={unfollowMutation}
          wavePending={isWavePending}
        />
      ) : null}

      {/* Tab-independent restart badge — visible on every tab so the user
          sees it on a plain Info-tab open, not only on the Runtime tab.
          Fixes the side-panel badge inconsistency (info-tab default = badge invisible
          before this change). */}
      {managedAgent?.needsRestart ? (
        <RestartDiffBadge
          autoRestartEnabled={managedAgent.autoRestartOnConfigChange}
          className="self-center"
          restartDiff={managedAgent.restartDiff}
        />
      ) : null}

      {showTabSection ? (
        <section className={cn(isBot ? "space-y-0" : "space-y-3")}>
          {showTabBar ? (
            <div
              className={cn(
                isBot &&
                  "sticky z-40 -mx-4 px-4 pb-3 pt-2 motion-reduce:transition-none",
              )}
              data-testid={isBot ? "user-profile-sticky-tabs" : undefined}
              ref={setStickyTabsElement}
              style={
                isBot
                  ? {
                      top: "var(--buzz-profile-sticky-hero-height, 0px)",
                    }
                  : undefined
              }
            >
              <ProfileTabBar
                activeTab={activeTab}
                onTabChange={onTabChange}
                tabs={tabs}
              />
            </div>
          ) : null}
          <ProfileTabContentTransition
            activeTab={activeTab}
            className={cn(showTabBar && "pt-2")}
            tabs={tabIds}
          >
            {activeTab === "info" ? (
              <ProfileInfoTabContent
                activeTurns={activeTurns}
                activityAgent={activityAgent}
                agentInfoFields={agentInfoFields}
                archiveActions={archiveActions}
                canArchiveAgent={isBot && archiveActions.canArchive}
                canDeleteAgent={canDeleteAgent}
                channelIdToName={channelIdToName}
                isArchived={isArchived}
                isDeleteAgentPending={isAgentActionPending}
                managedAgent={managedAgent}
                onEditAgent={handleEditAgent}
                onCreateCard={onCreateCard}
                onDeleteAgent={onDeleteAgent}
                onDuplicateAgent={onDuplicateAgent}
                onExportAgent={onExportAgent}
                onOpenActivity={onOpenActivity}
                pubkey={pubkey}
                showActivityIngress={showActivityIngress}
                showInstructionBlock={showInstructionBlock}
              />
            ) : null}
            {activeTab === "runtime" ? (
              <div className="space-y-4">
                <ProfileRuntimeTabContent
                  autoRestartEnabled={
                    managedAgent?.autoRestartOnConfigChange ?? false
                  }
                  currentPubkey={pubkey}
                  diagnosticsFields={diagnosticsFields}
                  diagnosticsSummary={diagnosticsTrailing}
                  configurationFields={runtimeFields}
                  instances={instanceBuckets.live}
                  archivedInstances={instanceBuckets.archived}
                  modelSettings={
                    isOwner === true && managedAgent !== undefined ? (
                      <AgentConfigPanel
                        advancedMode="flat"
                        onEdit={canEditAgent ? handleEditAgent : undefined}
                        pubkey={managedAgent.pubkey}
                        sections={["model"]}
                      />
                    ) : undefined
                  }
                  needsRestart={managedAgent?.needsRestart ?? false}
                  onToggleStartOnLaunch={
                    managedAgent?.backend.type === "local"
                      ? handleToggleAgentAutoStart
                      : undefined
                  }
                  restartDiff={managedAgent?.restartDiff ?? []}
                  startOnLaunchEnabled={managedAgent?.startOnAppLaunch}
                  startOnLaunchPending={isAgentActionPending}
                  onOpenDiagnostics={onOpenDiagnostics}
                  onOpenInstance={onOpenInstance}
                  showDiagnosticsIngress={showDiagnosticsIngress}
                />
                {isOwner === true && managedAgent !== undefined ? (
                  <AgentConfigPanel
                    advancedMode="flat"
                    pubkey={managedAgent.pubkey}
                    sections={["mcp", "advanced"]}
                  />
                ) : null}
              </div>
            ) : null}
            {activeTab === "channels" ? (
              <ChannelsFocusedView
                canAddToChannel={canAddToChannel}
                channels={channels}
                isActionPending={isAgentActionPending}
                isLoading={channelsLoading}
                onAddToChannel={onAddToChannel}
                onOpenChannel={onOpenChannel}
                variant="embedded"
              />
            ) : null}
            {activeTab === "memories" && pubkey ? (
              <MemoryFocusedView
                agentPubkey={pubkey}
                variant="embedded"
                viewerIsOwner={isOwner}
              />
            ) : null}
          </ProfileTabContentTransition>
        </section>
      ) : null}
    </div>
  );
}

// ── Hero & metadata ──────────────────────────────────────────────────────────

function ProfileHero({
  notManagedOnDevice,
  displayName,
  isBot,
  onEditAgent,
  presenceStatus,
  profile,
  userStatus,
}: {
  displayName: string;
  notManagedOnDevice?: boolean;
  isBot: boolean;
  onEditAgent?: () => void;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ProfileSummaryViewProps["profile"];
  userStatus: ProfileSummaryViewProps["userStatus"];
}) {
  const presenceDotClassName = isBot ? "h-4.5 w-4.5" : "h-3.5 w-3.5";
  const botIndicator = isBot ? (
    <BotIdenticon
      className="shrink-0 rounded"
      data-testid="profile-bot-indicator"
      size={20}
      value={displayName}
    />
  ) : null;

  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <MaskedAvatarBadgeFrame
        badge={
          presenceStatus ? (
            <span
              aria-label={getPresenceLabel(presenceStatus)}
              className="flex h-6 w-6 items-center justify-center rounded-full"
              data-testid="user-profile-presence-badge"
              role="img"
            >
              <PresenceDot
                className={presenceDotClassName}
                status={presenceStatus}
              />
            </span>
          ) : null
        }
        badgeBox={PROFILE_HERO_PRESENCE_BADGE.shell}
        className="h-20 w-20"
        cornerRadius={isBot ? 24 : undefined}
        curve={STATUS_DOT_MASK_CURVE}
        cutout={PROFILE_HERO_PRESENCE_BADGE.cutout}
        size={80}
      >
        <ProfileAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          className="h-full w-full text-xl"
          iconClassName="h-8 w-8"
          label={displayName}
          plain
          shape={isBot ? "squircle" : "circle"}
          testId="user-profile-avatar"
        />
      </MaskedAvatarBadgeFrame>

      <div className="flex flex-col items-center gap-1">
        {onEditAgent ? (
          <h3 className="max-w-full" data-testid="user-profile-name-row">
            <button
              aria-label={`Edit ${displayName}`}
              className="group relative flex max-w-full items-center justify-center gap-2 rounded-lg px-1 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              data-testid="user-profile-edit-agent"
              onClick={onEditAgent}
              type="button"
            >
              <span className="truncate text-xl font-semibold tracking-tight">
                {displayName}
              </span>
              {botIndicator}
              {notManagedOnDevice ? <OtherSetupAgentMarker /> : null}
              <span
                aria-hidden="true"
                className="pointer-events-none absolute top-1/2 left-full ml-1 -translate-y-1/2 text-muted-foreground opacity-0 transition-[color,opacity] duration-150 ease-out group-hover:text-foreground group-hover:opacity-100 group-focus-visible:opacity-100"
                data-testid="user-profile-edit-agent-icon"
              >
                <Pencil className="h-4 w-4" />
              </span>
            </button>
          </h3>
        ) : (
          <h3
            className="flex max-w-full items-center justify-center gap-2 text-xl font-semibold tracking-tight"
            data-testid="user-profile-name-row"
          >
            <span className="truncate">{displayName}</span>
            {botIndicator}
            {notManagedOnDevice ? <OtherSetupAgentMarker /> : null}
          </h3>
        )}

        {profile?.about?.trim() ? (
          <ProfileHeroDescription
            about={profile.about.trim()}
            key={profile.about.trim()}
          />
        ) : null}

        {profile?.nip05Handle ? (
          <p className="text-sm text-muted-foreground">{profile.nip05Handle}</p>
        ) : null}

        {userStatus && (userStatus.text || userStatus.emoji) ? (
          <p className="text-sm text-muted-foreground">
            {userStatus.emoji ? (
              <StatusEmoji
                className="mr-1 inline h-3.5 w-3.5"
                value={userStatus.emoji}
              />
            ) : null}
            {userStatus.text}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function ProfileHeroDescription({ about }: { about: string }) {
  const [expanded, setExpanded] = React.useState(false);
  const [isTruncated, setIsTruncated] = React.useState(false);
  const textRef = React.useRef<HTMLParagraphElement>(null);

  const measureTruncation = React.useCallback(() => {
    const element = textRef.current;
    if (!element || expanded) {
      return;
    }
    setIsTruncated(element.scrollHeight > element.clientHeight + 1);
  }, [expanded]);

  React.useLayoutEffect(() => {
    measureTruncation();
  }, [measureTruncation]);

  React.useEffect(() => {
    const element = textRef.current;
    if (!element) {
      return;
    }

    const observer = new ResizeObserver(() => {
      measureTruncation();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [measureTruncation]);

  const toggleClassName =
    "inline-flex items-center gap-0.5 text-xs font-medium text-muted-foreground opacity-60 transition-opacity hover:text-foreground hover:opacity-100";

  return (
    <div className="flex w-full flex-col items-center gap-0.5">
      <div className="w-fit max-w-full px-2">
        <p
          className={cn(
            "text-center whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground",
            !expanded && "line-clamp-3",
          )}
          data-testid="user-profile-description"
          ref={textRef}
        >
          {about}
        </p>
      </div>
      {!expanded && isTruncated ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(true)}
          type="button"
        >
          more
          <ChevronDown className="h-4 w-4" />
        </button>
      ) : null}
      {expanded ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(false)}
          type="button"
        >
          less
          <ChevronUp className="h-4 w-4" />
        </button>
      ) : null}
    </div>
  );
}
