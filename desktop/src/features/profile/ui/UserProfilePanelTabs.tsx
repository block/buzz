import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Archive,
  ChevronRight,
  Info,
  RefreshCw,
  Wrench,
} from "lucide-react";

import type { ManagedAgent } from "@/shared/api/types";
import type { ActiveTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import {
  AgentDetailsRows,
  AgentInstructionRow,
} from "@/features/profile/ui/UserProfilePanelAgentDetails";
import type { ProfileActivityAgent } from "@/features/profile/lib/profileActivityAgent";
import { resolveActivityChannelId } from "@/features/profile/lib/profileActivityCarousel";
import {
  type ProfileActivityFeedScope,
  useProfileActivityFeedScope,
} from "@/features/profile/lib/profileActivityFeedScope";
import {
  type ProfileField,
  ProfileFieldGroup,
  ProfileFieldRows,
} from "@/features/profile/ui/UserProfilePanelFields";
import type { ProfilePanelTab } from "@/features/profile/ui/UserProfilePanelUtils";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
import {
  Carousel,
  type CarouselApi,
  CarouselContent,
  CarouselItem,
} from "@/shared/ui/carousel";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function ProfileIngressRow({
  disabled,
  icon: Icon,
  label,
  onClick,
  testId,
  trailing,
}: {
  disabled?: boolean;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId: string;
  trailing?: React.ReactNode;
}) {
  const trailingTitle = typeof trailing === "string" ? trailing : undefined;

  return (
    <button
      className="flex w-full items-center gap-3 rounded-2xl bg-muted/20 px-4 py-2 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-50"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-sm font-medium text-foreground">
        {label}
      </span>
      {trailing ? (
        <span
          className="max-w-[45%] truncate text-right text-sm text-muted-foreground"
          title={trailingTitle}
        >
          {trailing}
        </span>
      ) : null}
      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

function useHorizontalDragScroll() {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const didDragRef = React.useRef(false);
  const momentumFrameRef = React.useRef<number | null>(null);
  const activeListenersRef = React.useRef<{
    move: (event: PointerEvent) => void;
    up: (event: PointerEvent) => void;
  } | null>(null);

  const stopMomentum = React.useCallback(() => {
    if (momentumFrameRef.current !== null) {
      cancelAnimationFrame(momentumFrameRef.current);
      momentumFrameRef.current = null;
    }
  }, []);

  const cleanupListeners = React.useCallback(() => {
    const active = activeListenersRef.current;
    if (!active) {
      return;
    }

    window.removeEventListener("pointermove", active.move);
    window.removeEventListener("pointerup", active.up);
    window.removeEventListener("pointercancel", active.up);
    activeListenersRef.current = null;
  }, []);

  React.useEffect(() => {
    return () => {
      cleanupListeners();
      stopMomentum();
    };
  }, [cleanupListeners, stopMomentum]);

  const handlePointerDown = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const element = scrollRef.current;
      if (!element || event.button !== 0) {
        return;
      }

      cleanupListeners();
      stopMomentum();

      const startX = event.clientX;
      const startScrollLeft = element.scrollLeft;
      let lastX = event.clientX;
      let lastTime = performance.now();
      let velocity = 0;
      didDragRef.current = false;

      const handleMove = (moveEvent: PointerEvent) => {
        const now = performance.now();
        const deltaX = moveEvent.clientX - startX;
        if (!didDragRef.current && Math.abs(deltaX) > 4) {
          didDragRef.current = true;
        }

        if (didDragRef.current) {
          moveEvent.preventDefault();
          element.scrollLeft = startScrollLeft - deltaX;

          const dt = now - lastTime;
          if (dt > 0) {
            velocity = -(moveEvent.clientX - lastX) / dt;
          }
          lastX = moveEvent.clientX;
          lastTime = now;
        }
      };

      const handleUp = () => {
        cleanupListeners();
        window.setTimeout(() => {
          didDragRef.current = false;
        }, 0);

        const minVelocity = 0.02;
        if (!didDragRef.current || Math.abs(velocity) < minVelocity) {
          return;
        }

        let frameTime = performance.now();
        const frictionPerMs = 0.004;

        const step = (now: number) => {
          const dt = now - frameTime;
          frameTime = now;

          const maxScroll = element.scrollWidth - element.clientWidth;
          element.scrollLeft = Math.max(
            0,
            Math.min(maxScroll, element.scrollLeft + velocity * dt),
          );

          if (element.scrollLeft <= 0 || element.scrollLeft >= maxScroll) {
            momentumFrameRef.current = null;
            return;
          }

          velocity *= Math.exp(-frictionPerMs * dt);
          if (Math.abs(velocity) >= minVelocity) {
            momentumFrameRef.current = requestAnimationFrame(step);
          } else {
            momentumFrameRef.current = null;
          }
        };

        momentumFrameRef.current = requestAnimationFrame(step);
      };

      activeListenersRef.current = { move: handleMove, up: handleUp };
      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", handleUp);
      window.addEventListener("pointercancel", handleUp);
    },
    [cleanupListeners, stopMomentum],
  );

  return {
    didDragRef,
    onPointerDown: handlePointerDown,
    scrollRef,
  };
}

export function ProfileTabBar({
  activeTab,
  onTabChange,
  tabs,
}: {
  activeTab: ProfilePanelTab;
  onTabChange: (tab: ProfilePanelTab) => void;
  tabs: Array<{
    id: ProfilePanelTab;
    label: string;
    trailing?: React.ReactNode;
  }>;
}) {
  const { didDragRef, onPointerDown, scrollRef } = useHorizontalDragScroll();

  return (
    <div
      className="-mx-4 cursor-grab select-none overflow-x-auto px-4 scrollbar-none active:cursor-grabbing [&::-webkit-scrollbar]:hidden"
      onPointerDown={onPointerDown}
      ref={scrollRef}
    >
      <div
        aria-label="Profile sections"
        className="flex w-max min-w-full justify-center gap-1.5"
        role="tablist"
      >
        {tabs.map((tab) => {
          const isActive = activeTab === tab.id;

          return (
            <Button
              aria-selected={isActive}
              className="shrink-0 rounded-full"
              data-testid={`user-profile-tab-${tab.id}`}
              key={tab.id}
              onClick={() => {
                if (didDragRef.current) {
                  return;
                }
                onTabChange(tab.id);
              }}
              role="tab"
              size="sm"
              type="button"
              variant={isActive ? "secondary" : "ghost"}
            >
              {tab.label}
              {tab.trailing ? (
                <span
                  className={cn(
                    "inline-flex items-center leading-none text-2xs",
                    isActive
                      ? "text-secondary-foreground/80"
                      : "text-muted-foreground",
                  )}
                >
                  {tab.trailing}
                </span>
              ) : null}
            </Button>
          );
        })}
      </div>
    </div>
  );
}

export function ProfileInfoTabContent({
  activeTurns,
  activityAgent,
  agentInfoFields,
  callerChannelId,
  channelIdToName,
  instances,
  isArchived,
  onOpenActivity,
  onOpenInstance,
  pubkey,
  showActivityIngress,
}: {
  activeTurns: ActiveTurnSummary[];
  activityAgent: ProfileActivityAgent | null;
  agentInfoFields: ProfileField[];
  callerChannelId: string | null;
  channelIdToName: Record<string, string>;
  instances: ManagedAgent[];
  isArchived: boolean;
  onOpenActivity: (channelId?: string | null) => void;
  onOpenInstance: (pubkey: string) => void;
  pubkey: string | null;
  showActivityIngress: boolean;
}) {
  const infoFields: ProfileField[] = isArchived
    ? [
        ...agentInfoFields,
        {
          displayValue: "Archived",
          icon: Archive,
          label: "Visibility",
          testId: "user-profile-archived-flair",
          trailingNode: <ArchiveStatusTooltip />,
        },
      ]
    : agentInfoFields;
  const hasInfoFields = infoFields.length > 0;
  const hasInstances = instances.length > 1;
  const feedScope = useProfileActivityFeedScope(activityAgent, activeTurns);
  const showLiveActivityEmbed =
    showActivityIngress && (feedScope.isLive || feedScope.hasFeedContent);

  if (!hasInfoFields && !showActivityIngress && !hasInstances) {
    return null;
  }

  return (
    <div className="space-y-2">
      {showActivityIngress ? (
        showLiveActivityEmbed && activityAgent ? (
          <ProfileLiveActivityEmbed
            activeTurns={activeTurns}
            activityAgent={activityAgent}
            callerChannelId={callerChannelId}
            channelIdToName={channelIdToName}
            feedScope={feedScope}
            onOpenActivity={onOpenActivity}
          />
        ) : (
          <ProfileIngressRow
            icon={Wrench}
            label="Activity log"
            onClick={() => onOpenActivity(null)}
            testId={`user-profile-view-activity-${pubkey}`}
            trailing="View"
          />
        )
      ) : null}
      {hasInfoFields ? <ProfileFieldGroup fields={infoFields} /> : null}
      {hasInstances ? (
        <ProfileInstancesSection
          currentPubkey={pubkey}
          instances={instances}
          onOpenInstance={onOpenInstance}
        />
      ) : null}
    </div>
  );
}

function ProfileInstancesSection({
  currentPubkey,
  instances,
  onOpenInstance,
}: {
  currentPubkey: string | null;
  instances: ManagedAgent[];
  onOpenInstance: (pubkey: string) => void;
}) {
  const [expanded, setExpanded] = React.useState(false);

  return (
    <div className="overflow-hidden rounded-2xl bg-muted/20">
      <button
        aria-expanded={expanded}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
        data-testid="user-profile-instances"
        onClick={() => setExpanded((value) => !value)}
        type="button"
      >
        <span className="min-w-0 flex-1 text-sm font-medium">Instances</span>
        <span className="text-sm text-muted-foreground">
          {instances.length}
        </span>
        <ChevronRight
          className={cn(
            "h-4 w-4 shrink-0 text-muted-foreground transition-transform",
            expanded && "rotate-90",
          )}
        />
      </button>
      {expanded ? (
        <div className="border-t border-border/60 px-2 py-2">
          {instances.map((instance) => {
            const isCurrent = instance.pubkey === currentPubkey;
            return (
              <button
                className="flex w-full items-center gap-3 rounded-xl px-3 py-2 text-left transition-colors hover:bg-muted/40"
                data-testid={`user-profile-instance-${instance.pubkey}`}
                key={instance.pubkey}
                onClick={() => onOpenInstance(instance.pubkey)}
                type="button"
              >
                <span className="min-w-0 flex-1 truncate text-sm font-medium">
                  {instance.name}
                </span>
                <span className="text-xs capitalize text-muted-foreground">
                  {isCurrent ? "Current" : instance.status.replace("_", " ")}
                </span>
                <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function ProfileLiveActivityEmbed({
  activeTurns,
  activityAgent,
  callerChannelId,
  channelIdToName,
  feedScope,
  onOpenActivity,
}: {
  activeTurns: ActiveTurnSummary[];
  activityAgent: ProfileActivityAgent;
  callerChannelId: string | null;
  channelIdToName: Record<string, string>;
  feedScope: ProfileActivityFeedScope;
  onOpenActivity: (channelId?: string | null) => void;
}) {
  const [carouselApi, setCarouselApi] = React.useState<CarouselApi>();
  const [selectedChannelId, setSelectedChannelId] = React.useState<
    string | null
  >(null);
  const [mountedChannelIds, setMountedChannelIds] = React.useState<Set<string>>(
    () => new Set(),
  );

  const slides = React.useMemo(() => {
    const channelIds = feedScope.isLive
      ? activeTurns.map((turn) => turn.channelId)
      : feedScope.channelIds;
    return [...new Set(channelIds)];
  }, [activeTurns, feedScope.channelIds, feedScope.isLive]);

  const activeChannelId = resolveActivityChannelId(
    slides,
    selectedChannelId,
    callerChannelId ?? feedScope.preferredChannelId,
  );
  const selectedIndex = activeChannelId ? slides.indexOf(activeChannelId) : 0;

  React.useEffect(() => {
    if (!carouselApi || !activeChannelId) {
      return;
    }

    const syncSelectedChannel = () => {
      setSelectedChannelId(slides[carouselApi.selectedScrollSnap()] ?? null);
    };

    carouselApi.on("select", syncSelectedChannel);
    carouselApi.on("reInit", syncSelectedChannel);

    return () => {
      carouselApi.off("select", syncSelectedChannel);
      carouselApi.off("reInit", syncSelectedChannel);
    };
  }, [activeChannelId, carouselApi, slides]);

  React.useEffect(() => {
    if (!carouselApi || !activeChannelId) {
      return;
    }

    const targetIndex = slides.indexOf(activeChannelId);
    if (targetIndex >= 0 && carouselApi.selectedScrollSnap() !== targetIndex) {
      carouselApi.scrollTo(targetIndex, true);
    }
    setSelectedChannelId(activeChannelId);
  }, [activeChannelId, carouselApi, slides]);

  React.useEffect(() => {
    if (!activeChannelId) {
      return;
    }

    setMountedChannelIds((current) => {
      if (current.has(activeChannelId)) {
        return current;
      }
      const next = new Set(current);
      next.add(activeChannelId);
      return next;
    });
  }, [activeChannelId]);

  const selectedTurn = feedScope.isLive
    ? (activeTurns.find((turn) => turn.channelId === activeChannelId) ??
      activeTurns[0] ??
      null)
    : null;
  const activeChannelName = activeChannelId
    ? (channelIdToName[activeChannelId] ?? activeChannelId)
    : null;
  const lastLiveAt =
    (activeChannelId
      ? feedScope.latestActivityAtByChannel[activeChannelId]
      : undefined) ??
    selectedTurn?.anchorAt ??
    null;
  const emptyState = feedScope.isLive ? "loading" : "idle";
  const emptyDescription = "Live activity will appear here.";
  const openSelectedActivity = React.useCallback(() => {
    onOpenActivity(activeChannelId);
  }, [activeChannelId, onOpenActivity]);

  const handleDotSelect = React.useCallback(
    (index: number) => {
      const targetIndex =
        slides.length === 2 && index === selectedIndex
          ? (selectedIndex + 1) % slides.length
          : index;
      carouselApi?.scrollTo(targetIndex);
    },
    [carouselApi, selectedIndex, slides.length],
  );

  if (slides.length === 0) {
    return (
      <section
        aria-label={`Open activity feed. Last live ${formatLastLiveLabel(lastLiveAt, Date.now())}.`}
        className="relative flex h-56 cursor-pointer flex-col overflow-hidden rounded-2xl border bg-background text-left shadow-none transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        data-testid={`user-profile-live-activity-${activityAgent.pubkey}`}
      >
        <button
          aria-label={`Open activity feed. Last live ${formatLastLiveLabel(lastLiveAt, Date.now())}.`}
          className="absolute inset-0 z-10 rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={openSelectedActivity}
          type="button"
        />
        <LiveActivityOpenButton
          activeChannelId={activeChannelId}
          lastLiveAt={lastLiveAt}
          onOpenActivity={onOpenActivity}
        />
        <ManagedAgentSessionPanel
          agent={activityAgent}
          autoTail={true}
          channelId={callerChannelId}
          className="relative z-0 min-h-0 flex-1 border-0 bg-transparent px-4 text-xs shadow-none **:data-message-id:pointer-events-none"
          emptyDescription={emptyDescription}
          emptyState={emptyState}
          panelPadding={false}
          rawLayout="responsive"
          showHeader={false}
          showRaw={false}
          transcriptContentClassName="py-4"
          transcriptVariant="compactPreview"
        />
        <div className="pointer-events-none absolute inset-0 z-20">
          <div className="absolute inset-x-0 bottom-0 flex flex-col items-start bg-linear-to-t from-background via-background/90 to-transparent px-3 pb-3 pt-24">
            <div className="min-w-0">
              <span className="block text-sm font-semibold text-muted-foreground">
                Latest Activity
              </span>
            </div>
          </div>
        </div>
      </section>
    );
  }

  return (
    <div>
      <section
        aria-label={`Open activity feed. Last live ${formatLastLiveLabel(lastLiveAt, Date.now())}.`}
        className="relative flex h-56 cursor-pointer flex-col overflow-hidden rounded-2xl border bg-background text-left shadow-none transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        data-testid={`user-profile-live-activity-${activityAgent.pubkey}`}
      >
        <button
          aria-label={`Open activity feed. Last live ${formatLastLiveLabel(lastLiveAt, Date.now())}.`}
          className="absolute inset-0 z-10 rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={openSelectedActivity}
          type="button"
        />
        <LiveActivityOpenButton
          activeChannelId={activeChannelId}
          lastLiveAt={lastLiveAt}
          onOpenActivity={onOpenActivity}
        />
        <Carousel
          className="relative z-0 flex min-h-0 flex-1 flex-col"
          opts={{
            align: "start",
            containScroll: "trimSnaps",
            dragFree: false,
            watchDrag: false,
          }}
          setApi={setCarouselApi}
        >
          <CarouselContent className="ml-0 h-full flex-1">
            {slides.map((channelId) => {
              const isMounted = mountedChannelIds.has(channelId);

              return (
                <CarouselItem
                  className="h-full basis-full pl-0"
                  data-mounted={isMounted ? "true" : "false"}
                  data-testid={`user-profile-activity-slide-${channelId}`}
                  key={channelId}
                >
                  {isMounted ? (
                    <ManagedAgentSessionPanel
                      agent={activityAgent}
                      autoTail={true}
                      channelId={channelId}
                      className="h-full min-h-0 border-0 bg-transparent px-4 text-xs shadow-none **:data-message-id:pointer-events-none"
                      emptyDescription={emptyDescription}
                      emptyState={emptyState}
                      panelPadding={false}
                      rawLayout="responsive"
                      showHeader={false}
                      showRaw={false}
                      transcriptContentClassName="py-4"
                      transcriptVariant="compactPreview"
                    />
                  ) : (
                    <div aria-hidden="true" className="h-full" />
                  )}
                </CarouselItem>
              );
            })}
          </CarouselContent>
        </Carousel>
        <div className="pointer-events-none absolute inset-0 z-20">
          <div className="absolute inset-x-0 bottom-0 flex flex-col items-start bg-linear-to-t from-background via-background/80 to-transparent px-3 pb-3 pt-16">
            <div className="min-w-0">
              <span className="block text-xs font-semibold text-muted-foreground">
                Latest Activity
              </span>
              {activeChannelName ? (
                <span
                  className="block truncate text-xs font-medium text-muted-foreground/75"
                  data-testid="user-profile-activity-channel-label"
                  title={`#${activeChannelName}`}
                >
                  #{activeChannelName}
                </span>
              ) : null}
            </div>
          </div>
        </div>
      </section>
      <ActivityCarouselDots
        channelIdToName={channelIdToName}
        onSelect={handleDotSelect}
        selectedIndex={selectedIndex}
        slides={slides}
      />
    </div>
  );
}

function ActivityCarouselDots({
  channelIdToName,
  onSelect,
  selectedIndex,
  slides,
}: {
  channelIdToName: Record<string, string>;
  onSelect: (index: number) => void;
  selectedIndex: number;
  slides: string[];
}) {
  if (slides.length <= 1) {
    return null;
  }

  return (
    <div
      aria-label="Choose active channel feed"
      className="mt-2 flex items-center justify-center gap-1.5"
      role="tablist"
    >
      {slides.map((channelId, index) => {
        const isSelected = index === selectedIndex;
        const channelName = channelIdToName[channelId] ?? channelId;

        return (
          <button
            aria-label={`Show #${channelName} activity`}
            aria-selected={isSelected}
            className="group relative flex items-center justify-center before:absolute before:-inset-2 before:content-['']"
            data-testid={`user-profile-activity-dot-${channelId}`}
            key={channelId}
            onClick={(event) => {
              event.stopPropagation();
              onSelect(index);
            }}
            role="tab"
            type="button"
          >
            <span
              aria-hidden="true"
              className={cn(
                "relative z-10 block rounded-full bg-foreground transition-all",
                isSelected
                  ? "h-1 w-4"
                  : "h-1 w-1 opacity-30 group-hover:opacity-60",
              )}
            />
          </button>
        );
      })}
    </div>
  );
}

function LiveActivityOpenButton({
  activeChannelId,
  lastLiveAt,
  onOpenActivity,
}: {
  activeChannelId: string | null;
  lastLiveAt: number | null;
  onOpenActivity: (channelId?: string | null) => void;
}) {
  const now = useNow(15_000);
  const label = formatLastLiveLabel(lastLiveAt, now);

  return (
    <Button
      aria-label={`Open full activity. Last live ${label}.`}
      className="absolute right-3 top-3 z-40 rounded-full bg-primary px-2.5 text-xs font-semibold text-primary-foreground hover:bg-primary/90"
      onClick={(event) => {
        event.stopPropagation();
        onOpenActivity(activeChannelId);
      }}
      size="xs"
      title={`Last live ${label}`}
      type="button"
    >
      {label}
    </Button>
  );
}

function formatLastLiveLabel(timestamp: number | null, now: number): string {
  if (timestamp === null) {
    return "No activity yet";
  }

  const elapsedMs = Math.max(0, now - timestamp);
  const totalSeconds = Math.floor(elapsedMs / 1000);
  if (totalSeconds < 60) {
    return "Just now";
  }

  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return `${totalMinutes}m ago`;
  }

  const totalHours = Math.floor(totalMinutes / 60);
  if (totalHours < 24) {
    return `${totalHours}h ago`;
  }

  const totalDays = Math.floor(totalHours / 24);
  if (totalDays < 7) {
    return `${totalDays}d ago`;
  }

  const totalWeeks = Math.floor(totalDays / 7);
  return `${totalWeeks}w ago`;
}

function ArchiveStatusTooltip() {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label="What archived means"
          className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          data-testid="user-profile-archived-info"
          type="button"
        >
          <Info className="h-4 w-4" />
        </button>
      </TooltipTrigger>
      <TooltipContent align="end" className="max-w-72 text-left" side="top">
        <p className="text-sm">
          Archived agents do not appear in search, autocomplete, or member-add
          flows in this space. You can unarchive them at any time.
        </p>
      </TooltipContent>
    </Tooltip>
  );
}

export function ProfileRuntimeTabContent({
  agentInstruction,
  autoRestartEnabled = false,
  diagnosticsFields,
  diagnosticsSummary,
  needsRestart = false,
  onOpenDiagnostics,
  onOpenInstructions,
  runtimeConfigurationFields,
  runtimeSettingsFields,
  showDiagnosticsIngress,
  showInstructionBlock,
}: {
  agentInstruction: string | null;
  /** Whether the per-agent auto-restart toggle is ON. */
  autoRestartEnabled?: boolean;
  diagnosticsFields: ProfileField[];
  diagnosticsSummary: React.ReactNode;
  /** True when the running agent's config has drifted from what it was spawned with. */
  needsRestart?: boolean;
  onOpenDiagnostics: () => void;
  onOpenInstructions: () => void;
  runtimeConfigurationFields: ProfileField[];
  runtimeSettingsFields: ProfileField[];
  showDiagnosticsIngress: boolean;
  showInstructionBlock: boolean;
}) {
  const statusDiagnosticsFields = diagnosticsFields.filter(
    (field) => field.label === "Status",
  );
  const detailDiagnosticsFields = diagnosticsFields.filter(
    (field) => field.label !== "Last error" && field.label !== "Status",
  );
  const hasRuntimeRows =
    runtimeConfigurationFields.length > 0 || runtimeSettingsFields.length > 0;

  if (
    !hasRuntimeRows &&
    statusDiagnosticsFields.length === 0 &&
    detailDiagnosticsFields.length === 0 &&
    !showDiagnosticsIngress &&
    !showInstructionBlock
  ) {
    return null;
  }

  return (
    <div className="space-y-2">
      {needsRestart ? (
        <div
          className="flex items-start gap-3 rounded-2xl bg-amber-500/10 px-4 py-3"
          data-testid="needs-restart-banner"
        >
          <RefreshCw className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
          <div className="min-w-0 text-sm">
            <p className="font-medium text-amber-600 dark:text-amber-400">
              Restart required
            </p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {autoRestartEnabled
                ? "Configuration changed since this agent started. Buzz can restart it automatically after ~3 minutes idle, or stop and respawn it to apply now."
                : "Configuration changed since this agent started. Automatic restart is off for this agent \u2014 stop and respawn it to apply the changes."}
            </p>
          </div>
        </div>
      ) : null}
      {showInstructionBlock ? (
        <div className="overflow-hidden rounded-2xl bg-muted/20">
          <AgentInstructionRow
            instruction={agentInstruction}
            onOpenInstructions={onOpenInstructions}
          />
        </div>
      ) : null}
      {statusDiagnosticsFields.length > 0 ? (
        <ProfileFieldGroup fields={statusDiagnosticsFields} />
      ) : null}
      {showDiagnosticsIngress ? (
        <ProfileIngressRow
          icon={Activity}
          label="Harness Log"
          onClick={onOpenDiagnostics}
          testId="user-profile-diagnostics-ingress"
          trailing={diagnosticsSummary}
        />
      ) : null}
      {hasRuntimeRows ? (
        <div className="overflow-hidden rounded-2xl bg-muted/20">
          <AgentDetailsRows fields={runtimeConfigurationFields} />
          {runtimeSettingsFields.length > 0 ? (
            <ProfileFieldRows fields={runtimeSettingsFields} />
          ) : null}
        </div>
      ) : null}
      {detailDiagnosticsFields.length > 0 ? (
        <ProfileFieldGroup fields={detailDiagnosticsFields} />
      ) : null}
    </div>
  );
}
