import { ChevronDown, ClockFading, Hash } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { useTranslation } from "@/i18n";
import {
  channelLifecycle,
  channelLifecycleLabel,
} from "@/features/channels/lib/channelLifecycle";
import {
  DEFAULT_EPHEMERAL_TTL_SECONDS,
  formatTtlDuration,
} from "@/features/channels/lib/ephemeralChannel";
import { useIsProjectHomeChannel } from "@/features/projects/lib/projectHomeChannel";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { EditableInfoFieldRow } from "./ChannelManagementSheetRows";
import { ChannelTypePicker } from "./ChannelTypePicker";

const CHANNEL_TYPE_OPTIONS = [
  { value: "temporary", Icon: ClockFading },
  { value: "ongoing", Icon: Hash },
] as const;

/** Temporal TTL ids in picker order; labels resolve at render via `t()`. */
const EPHEMERAL_TIMEOUT_SECONDS: number[] = [
  30 * 60,
  60 * 60,
  6 * 60 * 60,
  12 * 60 * 60,
  24 * 60 * 60,
  3 * 24 * 60 * 60,
  DEFAULT_EPHEMERAL_TTL_SECONDS,
  14 * 24 * 60 * 60,
  30 * 24 * 60 * 60,
];

const CHANNEL_TYPE_RESIZE_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

export function ChannelTypeDetailRow({
  canEdit,
  channel,
  onEdit,
}: {
  canEdit: boolean;
  channel: Channel;
  onEdit?: () => void;
}) {
  const { t } = useTranslation();
  const projectHome = useIsProjectHomeChannel(channel.id);
  const lifecycle = channelLifecycle({
    projectHome,
    temporary: channel.ttlSeconds !== null,
  });

  return (
    <EditableInfoFieldRow
      editTestId="channel-management-edit-channel-type"
      label={t("channels.type.channel-type")}
      onEdit={canEdit ? onEdit : undefined}
      testId="channel-management-type"
      value={channelLifecycleLabel(lifecycle, channel.ttlSeconds)}
    />
  );
}

export function ChannelTypeSettings({
  channelId,
  disabled,
  label,
  onOpenChange,
  onTemporaryChange,
  onTtlSecondsChange,
  open,
  temporary,
  testIdPrefix,
  ttlSeconds,
  variant = "dropdown",
}: {
  channelId?: string | null;
  disabled?: boolean;
  label?: string;
  onOpenChange?: (open: boolean) => void;
  onTemporaryChange: (temporary: boolean) => void;
  onTtlSecondsChange: (ttlSeconds: number) => void;
  open?: boolean;
  temporary: boolean;
  testIdPrefix: string;
  ttlSeconds: number;
  variant?: "dropdown" | "segmented";
}) {
  const { t } = useTranslation();
  const projectHome = useIsProjectHomeChannel(channelId);
  const lifecycle = channelLifecycle({ projectHome, temporary });
  const shouldReduceMotion = useReducedMotion();
  const channelTypeResizeTransition = shouldReduceMotion
    ? { duration: 0 }
    : CHANNEL_TYPE_RESIZE_TRANSITION;
  const rowLabel = label ?? t("channels.type.channel-type");
  const currentDurationLabel = t("channels.type.current-duration", {
    duration: formatTtlDuration(ttlSeconds),
  });

  function ttlLabelForSeconds(seconds: number) {
    switch (seconds) {
      case 30 * 60:
        return t("channels.type.ttl-30-minutes");
      case 60 * 60:
        return t("channels.type.ttl-1-hour");
      case 6 * 60 * 60:
        return t("channels.type.ttl-6-hours");
      case 12 * 60 * 60:
        return t("channels.type.ttl-12-hours");
      case 24 * 60 * 60:
        return t("channels.type.ttl-1-day");
      case 3 * 24 * 60 * 60:
        return t("channels.type.ttl-3-days");
      case DEFAULT_EPHEMERAL_TTL_SECONDS:
        return t("channels.type.ttl-7-days");
      case 14 * 24 * 60 * 60:
        return t("channels.type.ttl-14-days");
      case 30 * 24 * 60 * 60:
        return t("channels.type.ttl-30-days");
      default:
        return currentDurationLabel;
    }
  }

  const timeoutDurationOptions = EPHEMERAL_TIMEOUT_SECONDS.map((seconds) => ({
    seconds,
    label: ttlLabelForSeconds(seconds),
  }));
  const channelTypeOptions = CHANNEL_TYPE_OPTIONS.map((option) => ({
    ...option,
    label:
      option.value === "temporary"
        ? t("channels.type.temporary")
        : t("channels.type.ongoing"),
  }));
  const selectedTimeoutOption = timeoutDurationOptions.find(
    (option) => option.seconds === ttlSeconds,
  );
  const timeoutOptions = selectedTimeoutOption
    ? timeoutDurationOptions
    : [
        { label: currentDurationLabel, seconds: ttlSeconds },
        ...timeoutDurationOptions,
      ];

  return (
    <div
      className="overflow-hidden rounded-xl border border-input bg-background"
      data-testid={`${testIdPrefix}-channel-type-container`}
    >
      <div
        className="flex items-center justify-between gap-3 px-3 py-3"
        data-testid={`${testIdPrefix}-channel-type-row`}
      >
        <span
          className={cn(
            "text-sm font-medium text-foreground",
            disabled && variant === "segmented" && "opacity-50",
          )}
        >
          {rowLabel}
        </span>
        {variant === "segmented" ? (
          <SegmentedControl
            disabled={disabled}
            legend={t("channels.type.channel-type")}
            onValueChange={(value) => onTemporaryChange(value === "temporary")}
            optionTestIdPrefix={`${testIdPrefix}-channel-type-option`}
            options={channelTypeOptions}
            testId={`${testIdPrefix}-channel-type`}
            value={temporary ? "temporary" : "ongoing"}
          />
        ) : (
          <ChannelTypePicker
            align="end"
            allowProject={projectHome}
            className="-mr-2.5"
            disabled={disabled}
            lifecycle={lifecycle}
            onLifecycleChange={(next) =>
              onTemporaryChange(next === "temporary")
            }
            onOpenChange={onOpenChange}
            open={open}
            testId={`${testIdPrefix}-channel-type`}
          />
        )}
      </div>
      <AnimatePresence initial={false}>
        {temporary && !projectHome ? (
          <motion.div
            animate={{ height: "auto", opacity: 1 }}
            className="overflow-hidden"
            exit={{ height: 0, opacity: 0 }}
            initial={{ height: 0, opacity: 0 }}
            key={`${testIdPrefix}-ephemeral-settings`}
            transition={channelTypeResizeTransition}
          >
            <div
              className="relative flex items-center justify-between gap-3 px-3 py-3 before:absolute before:inset-x-3 before:top-0 before:border-t before:border-border/70"
              data-testid={`${testIdPrefix}-ephemeral-settings`}
            >
              <label
                className={cn(
                  "text-sm font-medium",
                  disabled && variant === "segmented" && "opacity-50",
                )}
                htmlFor={`${testIdPrefix}-ttl`}
              >
                {t("channels.type.expires-after")}
              </label>
              <DropdownMenu modal={false}>
                <DropdownMenuTrigger asChild>
                  <Button
                    aria-label={t("channels.type.expires-after")}
                    className="-mr-2.5 ml-auto h-9 w-fit justify-end px-2.5 text-right text-sm font-medium text-foreground hover:bg-muted/50"
                    data-testid={`${testIdPrefix}-ttl`}
                    disabled={disabled}
                    id={`${testIdPrefix}-ttl`}
                    type="button"
                    variant="ghost"
                  >
                    <span className="text-right">
                      {selectedTimeoutOption?.label ?? currentDurationLabel}
                    </span>
                    <ChevronDown className="size-4 shrink-0 text-muted-foreground/70" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent
                  align="end"
                  onCloseAutoFocus={(event) => event.preventDefault()}
                  style={{
                    minWidth: "var(--radix-dropdown-menu-trigger-width)",
                  }}
                >
                  <DropdownMenuRadioGroup
                    onValueChange={(value) => onTtlSecondsChange(Number(value))}
                    value={String(ttlSeconds)}
                  >
                    {timeoutOptions.map((option) => (
                      <DropdownMenuRadioItem
                        data-testid={`${testIdPrefix}-ttl-option-${option.seconds}`}
                        key={option.seconds}
                        value={String(option.seconds)}
                      >
                        {option.label}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
