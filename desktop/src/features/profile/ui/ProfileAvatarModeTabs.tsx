import { createPortal } from "react-dom";

import type {
  AvatarEditorPresentation,
  AvatarMode,
} from "@/features/profile/ui/ProfileAvatarEditor.types";
import { cn } from "@/shared/lib/cn";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { useTranslation } from "@/i18n";

const MODE_TAB_ORDER: AvatarMode[] = ["image", "emoji", "animated"];

type ProfileAvatarModeTabsProps = {
  disabled: boolean;
  mode: AvatarMode;
  onModeChange: (mode: AvatarMode) => void;
  presentation: AvatarEditorPresentation;
  portalContainer?: HTMLElement | null;
};

export function ProfileAvatarModeTabs({
  disabled,
  mode,
  onModeChange,
  presentation,
  portalContainer,
}: ProfileAvatarModeTabsProps) {
  const { t } = useTranslation();
  const modeTabLabels: Record<AvatarMode, string> = {
    animated: t("profile.avatar-mode.animated"),
    emoji: t("profile.avatar-mode.emoji"),
    image: t("profile.avatar-mode.image"),
  };
  const segmentOptions = MODE_TAB_ORDER.map((value) => ({
    label: modeTabLabels[value],
    value,
  }));
  const isOnboardingModal = presentation === "onboarding-modal";
  const isOnboardingInline = presentation === "onboarding-inline";
  const tabs = isOnboardingInline ? (
    <SegmentedControl
      className="w-full bg-muted"
      disabled={disabled}
      indicatorTestId="onboarding-avatar-mode-indicator"
      legend={t("profile.avatar-mode.type-aria")}
      onValueChange={onModeChange}
      optionTestIdPrefix="onboarding-avatar-mode"
      options={segmentOptions}
      testId="onboarding-avatar-mode-control"
      value={mode}
    />
  ) : (
    <Tabs
      className={isOnboardingModal ? "flex w-full justify-center" : "w-full"}
      onValueChange={(nextMode) => {
        if (!disabled) onModeChange(nextMode as AvatarMode);
      }}
      value={mode}
    >
      <TabsList
        aria-label={t("profile.avatar-mode.type-aria")}
        className={cn(
          isOnboardingModal
            ? "relative isolate grid h-10 w-full max-w-[320px] grid-cols-3 overflow-hidden rounded-full bg-[color:rgb(var(--buzz-onboarding-avatar-control-fg)_/_0.12)] p-1 text-muted-foreground"
            : "relative isolate grid h-14 w-full grid-cols-3 overflow-hidden rounded-full bg-muted p-1 text-muted-foreground",
        )}
      >
        <div
          aria-hidden="true"
          className={cn(
            "absolute z-0 rounded-full transition-transform motion-reduce:transition-none",
            "bottom-1 left-1 top-1 shadow duration-[250ms] ease-out",
            isOnboardingModal
              ? "bg-[rgb(var(--buzz-onboarding-avatar-action-bg))]"
              : "bg-background",
          )}
          style={{
            transform: `translateX(${MODE_TAB_ORDER.indexOf(mode) * 100}%)`,
            width: "calc((100% - 8px) / 3)",
          }}
        />
        {MODE_TAB_ORDER.map((tabMode) => (
          <TabsTrigger
            className={cn(
              isOnboardingModal
                ? "relative z-10 h-full rounded-full bg-transparent px-4 text-sm font-normal shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:text-[rgb(var(--buzz-onboarding-avatar-action-fg))] data-[state=active]:shadow-none"
                : "relative z-10 h-full rounded-full bg-transparent text-sm font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none",
            )}
            disabled={disabled}
            key={tabMode}
            value={tabMode}
          >
            {modeTabLabels[tabMode]}
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
