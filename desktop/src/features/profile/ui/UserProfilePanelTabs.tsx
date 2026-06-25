import * as React from "react";
import type { LucideIcon } from "lucide-react";
import { Activity, ChevronRight, Wrench } from "lucide-react";

import {
  AgentDetailsRows,
  AgentInstructionRow,
} from "@/features/profile/ui/UserProfilePanelAgentDetails";
import {
  type ProfileField,
  ProfileFieldGroup,
  ProfileFieldRows,
} from "@/features/profile/ui/UserProfilePanelFields";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

export type ProfileTab = "info" | "runtime" | "channels" | "memories";

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
  activeTab: ProfileTab;
  onTabChange: (tab: ProfileTab) => void;
  tabs: Array<{
    id: ProfileTab;
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
                    "text-2xs",
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
  agentInfoFields,
  agentInstruction,
  onOpenActivity,
  pubkey,
  showActivityIngress,
  showInstructionBlock,
}: {
  agentInfoFields: ProfileField[];
  agentInstruction: string | null;
  onOpenActivity: () => void;
  pubkey: string | null;
  showActivityIngress: boolean;
  showInstructionBlock: boolean;
}) {
  const hasInfoFields = agentInfoFields.length > 0;

  if (!showInstructionBlock && !hasInfoFields && !showActivityIngress) {
    return null;
  }

  return (
    <div className="space-y-2">
      {showInstructionBlock ? (
        <div className="overflow-hidden rounded-2xl bg-muted/20">
          <AgentInstructionRow instruction={agentInstruction} />
        </div>
      ) : null}
      {hasInfoFields ? <ProfileFieldGroup fields={agentInfoFields} /> : null}
      {showActivityIngress ? (
        <ProfileIngressRow
          icon={Wrench}
          label="Activity log"
          onClick={onOpenActivity}
          testId={`user-profile-view-activity-${pubkey}`}
          trailing="View"
        />
      ) : null}
    </div>
  );
}

export function ProfileRuntimeTabContent({
  diagnosticsFields,
  diagnosticsSummary,
  managedAgent,
  modelLabel,
  onOpenDiagnostics,
  runtimeConfigurationFields,
  runtimeSettingsFields,
  showDiagnosticsIngress,
}: {
  diagnosticsFields: ProfileField[];
  diagnosticsSummary: React.ReactNode;
  managedAgent: ManagedAgent | undefined;
  modelLabel: string;
  onOpenDiagnostics: () => void;
  runtimeConfigurationFields: ProfileField[];
  runtimeSettingsFields: ProfileField[];
  showDiagnosticsIngress: boolean;
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
    !showDiagnosticsIngress
  ) {
    return null;
  }

  return (
    <div className="space-y-2">
      {statusDiagnosticsFields.length > 0 ? (
        <ProfileFieldGroup fields={statusDiagnosticsFields} />
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
      {showDiagnosticsIngress ? (
        <ProfileIngressRow
          icon={Activity}
          label="Diagnostics"
          onClick={onOpenDiagnostics}
          testId="user-profile-diagnostics-ingress"
          trailing={diagnosticsSummary}
        />
      ) : null}
    </div>
  );
}
