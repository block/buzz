import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import {
  DEFAULT_EPHEMERAL_TTL_SECONDS,
  formatTtlDuration,
} from "@/features/channels/lib/ephemeralChannel";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { ChannelTypePicker } from "./ChannelTypePicker";

const EPHEMERAL_TIMEOUT_OPTIONS = [
  { label: "30 minutes", seconds: 30 * 60 },
  { label: "1 hour", seconds: 60 * 60 },
  { label: "6 hours", seconds: 6 * 60 * 60 },
  { label: "12 hours", seconds: 12 * 60 * 60 },
  { label: "1 day", seconds: 24 * 60 * 60 },
  { label: "3 days", seconds: 3 * 24 * 60 * 60 },
  { label: "7 days", seconds: DEFAULT_EPHEMERAL_TTL_SECONDS },
  { label: "14 days", seconds: 14 * 24 * 60 * 60 },
  { label: "30 days", seconds: 30 * 24 * 60 * 60 },
] as const;

const CHANNEL_TYPE_RESIZE_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

export function ChannelTypeSettings({
  disabled,
  onOpenChange,
  onTemporaryChange,
  onTtlSecondsChange,
  open,
  temporary,
  testIdPrefix,
  ttlSeconds,
}: {
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
  onTemporaryChange: (temporary: boolean) => void;
  onTtlSecondsChange: (ttlSeconds: number) => void;
  open?: boolean;
  temporary: boolean;
  testIdPrefix: string;
  ttlSeconds: number;
}) {
  const shouldReduceMotion = useReducedMotion();
  const channelTypeResizeTransition = shouldReduceMotion
    ? { duration: 0 }
    : CHANNEL_TYPE_RESIZE_TRANSITION;
  const selectedTimeoutOption = EPHEMERAL_TIMEOUT_OPTIONS.find(
    (option) => option.seconds === ttlSeconds,
  );
  const timeoutOptions = selectedTimeoutOption
    ? EPHEMERAL_TIMEOUT_OPTIONS
    : [
        {
          label: `Current (${formatTtlDuration(ttlSeconds)})`,
          seconds: ttlSeconds,
        },
        ...EPHEMERAL_TIMEOUT_OPTIONS,
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
        <span className="text-sm font-medium text-foreground">
          Channel type
        </span>
        <ChannelTypePicker
          align="end"
          className="-mr-2.5"
          disabled={disabled}
          onOpenChange={onOpenChange}
          onTemporaryChange={onTemporaryChange}
          open={open}
          temporary={temporary}
          testId={`${testIdPrefix}-channel-type`}
        />
      </div>
      <AnimatePresence initial={false}>
        {temporary ? (
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
                className="text-sm font-medium"
                htmlFor={`${testIdPrefix}-ttl`}
              >
                Expires after
              </label>
              <DropdownMenu modal={false}>
                <DropdownMenuTrigger asChild>
                  <Button
                    aria-label="Expires after"
                    className="-mr-2.5 ml-auto h-9 w-fit justify-end px-2.5 text-right text-sm font-medium text-foreground hover:bg-muted/50"
                    data-testid={`${testIdPrefix}-ttl`}
                    disabled={disabled}
                    id={`${testIdPrefix}-ttl`}
                    type="button"
                    variant="ghost"
                  >
                    <span className="text-right">
                      {selectedTimeoutOption?.label ??
                        `Current (${formatTtlDuration(ttlSeconds)})`}
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
