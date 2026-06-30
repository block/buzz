import * as React from "react";
import type { LucideIcon } from "lucide-react";
import { Activity, Archive, ChevronRight, Info, Wrench } from "lucide-react";

import type { ActiveTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import {
  AgentDetailsRows,
  AgentInstructionRow,
} from "@/features/profile/ui/UserProfilePanelAgentDetails";
import {
  type ProfileField,
  ProfileFieldGroup,
  ProfileFieldRows,
} from "@/features/profile/ui/UserProfilePanelFields";
import type { ProfilePanelTab } from "@/features/profile/ui/UserProfilePanelUtils";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
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
  agentInfoFields,
  channelIdToName,
  isArchived,
  managedAgent,
  onOpenActivity,
  pubkey,
  showActivityIngress,
}: {
  activeTurns: ActiveTurnSummary[];
  agentInfoFields: ProfileField[];
  channelIdToName: Record<string, string>;
  isArchived: boolean;
  managedAgent?: ManagedAgent;
  onOpenActivity: (channelId?: string | null) => void;
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
  const showLiveActivityEmbed = showActivityIngress && activeTurns.length > 0;

  if (!hasInfoFields && !showActivityIngress) {
    return null;
  }

  return (
    <div className="space-y-2">
      {showActivityIngress ? (
        showLiveActivityEmbed && managedAgent ? (
          <ProfileLiveActivityEmbed
            activeTurns={activeTurns}
            channelIdToName={channelIdToName}
            managedAgent={managedAgent}
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
    </div>
  );
}

function ProfileLiveActivityEmbed({
  activeTurns,
  channelIdToName,
  managedAgent,
  onOpenActivity,
}: {
  activeTurns: ActiveTurnSummary[];
  channelIdToName: Record<string, string>;
  managedAgent: ManagedAgent;
  onOpenActivity: (channelId?: string | null) => void;
}) {
  const [selectedChannelId, setSelectedChannelId] = React.useState<
    string | null
  >(() => activeTurns[0]?.channelId ?? null);
  const now = useNow(1000);

  React.useEffect(() => {
    if (activeTurns.length === 0) {
      setSelectedChannelId(null);
      return;
    }

    if (!activeTurns.some((turn) => turn.channelId === selectedChannelId)) {
      setSelectedChannelId(activeTurns[0]?.channelId ?? null);
    }
  }, [activeTurns, selectedChannelId]);

  const selectedTurn =
    activeTurns.find((turn) => turn.channelId === selectedChannelId) ??
    activeTurns[0] ??
    null;
  const activeChannelId = selectedTurn?.channelId ?? null;
  const showSwitcher = activeTurns.length > 1;

  return (
    <section
      aria-label="Live activity preview"
      className="flex h-48 flex-col overflow-hidden rounded-2xl border border-border/60 bg-muted/20 text-left"
      data-testid={`user-profile-live-activity-${managedAgent.pubkey}`}
    >
      {showSwitcher ? (
        <div className="border-border/60 border-b px-3 py-2">
          <LiveActivityEmbedHeader
            activeChannelId={activeChannelId}
            elapsedLabel={
              selectedTurn ? formatElapsed(now - selectedTurn.anchorAt) : null
            }
            onOpenActivity={onOpenActivity}
          />
          <div
            aria-label="Choose active channel feed"
            className="mt-2 flex gap-1 overflow-x-auto pb-0.5"
            role="tablist"
          >
            {activeTurns.map((turn) => {
              const isSelected = turn.channelId === activeChannelId;
              const channelName =
                channelIdToName[turn.channelId] ?? turn.channelId;

              return (
                <button
                  aria-selected={isSelected}
                  className={cn(
                    "max-w-36 shrink-0 truncate rounded-full px-2.5 py-1 text-xs font-medium transition-colors",
                    isSelected
                      ? "bg-primary text-primary-foreground"
                      : "bg-muted/60 text-muted-foreground hover:bg-muted hover:text-foreground",
                  )}
                  key={turn.channelId}
                  onClick={() => setSelectedChannelId(turn.channelId)}
                  role="tab"
                  title={channelName}
                  type="button"
                >
                  #{channelName}
                </button>
              );
            })}
          </div>
        </div>
      ) : (
        <div className="border-border/60 border-b px-3 py-2">
          <LiveActivityEmbedHeader
            activeChannelId={activeChannelId}
            elapsedLabel={
              selectedTurn ? formatElapsed(now - selectedTurn.anchorAt) : null
            }
            onOpenActivity={onOpenActivity}
          />
        </div>
      )}
      <ManagedAgentSessionPanel
        agent={managedAgent}
        autoTail={true}
        channelId={activeChannelId}
        className="min-h-0 flex-1 rounded-none border-0 bg-transparent p-3 shadow-none"
        emptyDescription="Live activity will appear here."
        rawLayout="responsive"
        showHeader={false}
        showRaw={false}
      />
    </section>
  );
}

function LiveActivityEmbedHeader({
  activeChannelId,
  elapsedLabel,
  onOpenActivity,
}: {
  activeChannelId: string | null;
  elapsedLabel: string | null;
  onOpenActivity: (channelId?: string | null) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-2 text-xs">
      <span className="font-medium text-foreground">Live activity</span>
      <div className="flex shrink-0 items-center gap-2">
        {elapsedLabel ? (
          <span className="text-muted-foreground">{elapsedLabel}</span>
        ) : null}
        <Button
          className="h-6 px-2 text-2xs"
          onClick={() => onOpenActivity(activeChannelId)}
          type="button"
          variant="ghost"
        >
          Open full activity
        </Button>
      </div>
    </div>
  );
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
  diagnosticsFields,
  diagnosticsSummary,
  managedAgent,
  modelLabel,
  onOpenDiagnostics,
  onOpenInstructions,
  runtimeConfigurationFields,
  runtimeSettingsFields,
  showDiagnosticsIngress,
  showInstructionBlock,
}: {
  agentInstruction: string | null;
  diagnosticsFields: ProfileField[];
  diagnosticsSummary: React.ReactNode;
  managedAgent: ManagedAgent | undefined;
  modelLabel: string;
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
    runtimeConfigurationFields.length > 0 ||
    runtimeSettingsFields.length > 0 ||
    managedAgent !== undefined ||
    modelLabel.trim().length > 0;

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
          <AgentDetailsRows
            fields={runtimeConfigurationFields}
            managedAgent={managedAgent}
            modelLabel={modelLabel}
            showModel={true}
          />
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
