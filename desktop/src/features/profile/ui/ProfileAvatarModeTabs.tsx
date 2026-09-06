import { createPortal } from "react-dom";

import type {
  AvatarEditorPresentation,
  AvatarMode,
} from "@/features/profile/ui/ProfileAvatarEditor.types";
import { cn } from "@/shared/lib/cn";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";

const MODE_TAB_ORDER: AvatarMode[] = ["image", "emoji", "animated"];
const MODE_TAB_LABELS: Record<AvatarMode, string> = {
  animated: "Animated",
  emoji: "Emoji",
  image: "Image",
};

type ProfileAvatarModeTabsProps = {
  disabled: boolean;
  mode: AvatarMode;
  onModeChange: (mode: AvatarMode) => void;
  orientation?: "horizontal" | "vertical";
  presentation: AvatarEditorPresentation;
  portalContainer?: HTMLElement | null;
};

export function ProfileAvatarModeTabs({
  disabled,
  mode,
  onModeChange,
  orientation = "horizontal",
  presentation,
  portalContainer,
}: ProfileAvatarModeTabsProps) {
  const isOnboardingModal = presentation === "onboarding-modal";
  const isOnboardingInline = presentation === "onboarding-inline";
  const isOnboardingSurface = isOnboardingInline || isOnboardingModal;
  const isVertical = orientation === "vertical";
  const tabs = (
    <Tabs
      className={cn(
        "w-full",
        isOnboardingSurface && !isVertical && "flex justify-center",
      )}
      onValueChange={(nextMode) => {
        if (!disabled) onModeChange(nextMode as AvatarMode);
      }}
      value={mode}
    >
      <TabsList
        aria-label="Avatar type"
        className={cn(
          isOnboardingSurface && isVertical
            ? "flex h-auto w-full flex-col gap-1 rounded-xl bg-transparent p-0 text-muted-foreground"
            : isOnboardingSurface
              ? isOnboardingInline
                ? "relative isolate grid h-9 w-full grid-cols-3 gap-0.5 overflow-hidden rounded-full border border-[#d4d4d4] bg-[#e2e2e2]/30 p-[3px] text-[#0f0f0f]"
                : "relative isolate grid h-10 w-full max-w-[320px] grid-cols-3 overflow-hidden rounded-full bg-[color:rgb(var(--buzz-onboarding-avatar-control-fg)_/_0.12)] p-1 text-muted-foreground"
              : "relative isolate grid h-14 w-full grid-cols-3 overflow-hidden rounded-full bg-muted p-1 text-muted-foreground",
        )}
      >
        {!isVertical ? (
          <div
            aria-hidden="true"
            className={cn(
              "absolute z-0 rounded-full transition-transform motion-reduce:transition-none",
              isOnboardingInline
                ? "bottom-[3px] left-[3px] top-[3px] bg-[#e2e2e2] duration-200 ease-in-out"
                : cn(
                    "bottom-1 left-1 top-1 shadow duration-[250ms] ease-out",
                    isOnboardingSurface
                      ? "bg-[rgb(var(--buzz-onboarding-avatar-action-bg))]"
                      : "bg-background",
                  ),
            )}
            style={{
              transform: isOnboardingInline
                ? `translateX(calc(${MODE_TAB_ORDER.indexOf(mode)} * (100% + 2px)))`
                : `translateX(${MODE_TAB_ORDER.indexOf(mode) * 100}%)`,
              width: isOnboardingInline
                ? "calc((100% - 10px) / 3)"
                : "calc((100% - 8px) / 3)",
            }}
          />
        ) : null}
        {MODE_TAB_ORDER.map((tabMode) => (
          <TabsTrigger
            className={cn(
              isOnboardingSurface && isVertical
                ? "h-10 w-full justify-start rounded-lg bg-transparent px-3 text-sm font-medium shadow-none transition-colors duration-150 ease-out hover:bg-foreground/[0.05] data-[state=active]:bg-[rgb(var(--buzz-onboarding-avatar-action-bg))] data-[state=active]:text-[rgb(var(--buzz-onboarding-avatar-action-fg))] data-[state=active]:shadow-none"
                : isOnboardingSurface
                  ? isOnboardingInline
                    ? "relative z-10 h-full rounded-full bg-transparent px-4 py-1 text-sm font-medium text-[#0f0f0f] shadow-none transition-colors duration-150 ease-out focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-[#0f0f0f] focus-visible:ring-offset-0 data-[state=active]:bg-transparent data-[state=active]:text-[#0f0f0f] data-[state=active]:shadow-none motion-reduce:transition-none"
                    : "relative z-10 h-full rounded-full bg-transparent px-4 text-sm font-normal shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:text-[rgb(var(--buzz-onboarding-avatar-action-fg))] data-[state=active]:shadow-none"
                  : "relative z-10 h-full rounded-full bg-transparent text-sm font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none",
            )}
            disabled={disabled}
            key={tabMode}
            value={tabMode}
          >
            {MODE_TAB_LABELS[tabMode]}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );

  return portalContainer === undefined
    ? tabs
    : portalContainer
      ? createPortal(tabs, portalContainer)
      : null;
}
