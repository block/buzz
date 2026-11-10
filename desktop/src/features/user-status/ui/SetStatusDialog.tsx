import * as React from "react";
import {
  CalendarClock,
  Check,
  ChevronDown,
  Clock3,
  SmilePlus,
} from "lucide-react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import {
  DEFAULT_USER_STATUS_EMOJI,
  StatusEmoji,
} from "@/features/user-status/ui/StatusEmoji";
import type { UserStatusInput } from "@/features/user-status/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Calendar } from "@/shared/ui/calendar";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

const PRESETS = [
  { text: "In a meeting", emoji: "\uD83D\uDDE3\uFE0F" },
  { text: "Commuting", emoji: "\uD83D\uDE8C" },
  { text: "Out sick", emoji: "\uD83E\uDD12" },
  { text: "Vacationing", emoji: "\uD83C\uDFD6\uFE0F" },
  { text: "Working remotely", emoji: "\uD83C\uDFE0" },
] as const;

const DURATIONS = [
  "1 hour",
  "8 hours",
  "Today",
  "This week",
  "Custom",
] as const;

type DurationLabel = (typeof DURATIONS)[number];

const HALF_HOUR_TIMES = Array.from({ length: 48 }, (_, index) => {
  const hour = Math.floor(index / 2);
  const minute = index % 2 === 0 ? 0 : 30;
  const period = hour < 12 ? "AM" : "PM";
  const displayHour = hour % 12 || 12;
  return {
    label: `${displayHour}:${minute.toString().padStart(2, "0")} ${period}`,
    value: `${hour.toString().padStart(2, "0")}:${minute.toString().padStart(2, "0")}`,
  };
});

type SetStatusDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialText?: string;
  initialEmoji?: string;
  initialExpiresAt?: number;
  initialUpdatedAt?: number;
  onSave: (status: UserStatusInput) => void;
  onClear: () => void;
  hasExistingStatus: boolean;
};

function roundUpToHalfHour(value: Date) {
  const date = new Date(value);
  date.setSeconds(0, 0);
  date.setMinutes(Math.ceil(date.getMinutes() / 30) * 30);
  return date;
}

function defaultCustomDate() {
  return roundUpToHalfHour(new Date(Date.now() + 24 * 60 * 60 * 1_000));
}

function toLocalTimeValue(date: Date) {
  return `${date.getHours().toString().padStart(2, "0")}:${date.getMinutes().toString().padStart(2, "0")}`;
}

function formattedTime(date: Date) {
  return date.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

function endOfToday(from: Date) {
  const end = new Date(from);
  end.setHours(24, 0, 0, 0);
  return end;
}

function endOfWeek(from: Date) {
  const end = new Date(from);
  const daysUntilMonday = (8 - end.getDay()) % 7 || 7;
  end.setDate(end.getDate() + daysUntilMonday);
  end.setHours(0, 0, 0, 0);
  return end;
}

function inferredDuration(
  expiresAt: number | undefined,
  updatedAt: number | undefined,
): DurationLabel {
  if (!expiresAt || expiresAt * 1_000 <= Date.now()) return "Today";
  if (!updatedAt) return "Custom";

  const expiration = expiresAt * 1_000;
  const updated = updatedAt * 1_000;
  const withinTwoMinutes = (expected: number) =>
    Math.abs(expiration - expected) <= 2 * 60_000;
  if (withinTwoMinutes(updated + 60 * 60_000)) return "1 hour";
  if (withinTwoMinutes(updated + 8 * 60 * 60_000)) return "8 hours";
  if (withinTwoMinutes(endOfToday(new Date(updated)).getTime())) return "Today";
  if (withinTwoMinutes(endOfWeek(new Date(updated)).getTime())) {
    return "This week";
  }
  return "Custom";
}

function withCustomDate(current: Date, selected: Date) {
  const next = new Date(current);
  next.setFullYear(
    selected.getFullYear(),
    selected.getMonth(),
    selected.getDate(),
  );
  return next;
}

function startOfToday() {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  return today;
}

function formattedDate(date: Date) {
  return date.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

function withCustomTime(current: Date, value: string) {
  const [hour, minute] = value.split(":").map(Number);
  if (!Number.isFinite(hour) || !Number.isFinite(minute)) return current;
  const next = new Date(current);
  next.setHours(hour, minute, 0, 0);
  return next;
}

function StatusSection({
  label,
  children,
  className,
}: {
  label?: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("space-y-1.5", className)}>
      {label ? (
        <h3 className="px-1 text-xs font-medium text-muted-foreground">
          {label}
        </h3>
      ) : null}
      <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-muted/20">
        {children}
      </div>
    </section>
  );
}

const ROW_CLASS =
  "flex min-h-11 w-full items-center gap-3 px-3 py-2.5 text-left text-sm transition-colors hover:bg-muted/50 focus:outline-none focus-visible:bg-muted/50";

export function SetStatusDialog({
  open,
  onOpenChange,
  initialText = "",
  initialEmoji = "",
  initialExpiresAt,
  initialUpdatedAt,
  onSave,
  onClear,
  hasExistingStatus,
}: SetStatusDialogProps) {
  const [text, setText] = React.useState(initialText);
  const [emoji, setEmoji] = React.useState(initialEmoji);
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [calendarOpen, setCalendarOpen] = React.useState(false);
  const [duration, setDuration] = React.useState<DurationLabel>("Today");
  const [customUntil, setCustomUntil] = React.useState(defaultCustomDate);
  const [durationTouched, setDurationTouched] = React.useState(false);
  const [saveError, setSaveError] = React.useState("");
  const [baseline, setBaseline] = React.useState(() => ({
    text: initialText,
    emoji: initialEmoji,
    expiresAt: initialExpiresAt,
    duration: null as DurationLabel | null,
    customUntil: null as Date | null,
    hasExistingStatus,
  }));
  const initializedForOpenRef = React.useRef(false);

  React.useEffect(() => {
    if (!open) {
      initializedForOpenRef.current = false;
      return;
    }
    if (initializedForOpenRef.current) return;
    initializedForOpenRef.current = true;
    setText(initialText);
    setEmoji(initialEmoji);
    const currentExpiration = initialExpiresAt
      ? new Date(initialExpiresAt * 1_000)
      : null;
    if (currentExpiration && currentExpiration.getTime() > Date.now()) {
      const nextDuration = inferredDuration(initialExpiresAt, initialUpdatedAt);
      setDuration(nextDuration);
      setCustomUntil(currentExpiration);
      setBaseline({
        text: initialText,
        emoji: initialEmoji,
        expiresAt: initialExpiresAt,
        duration: nextDuration,
        customUntil: currentExpiration,
        hasExistingStatus,
      });
    } else {
      setDuration("Today");
      const nextCustomUntil = defaultCustomDate();
      setCustomUntil(nextCustomUntil);
      setBaseline({
        text: initialText,
        emoji: initialEmoji,
        expiresAt: undefined,
        duration: null,
        customUntil: null,
        hasExistingStatus,
      });
    }
    setDurationTouched(false);
    setSaveError("");
  }, [
    open,
    initialText,
    initialEmoji,
    initialExpiresAt,
    initialUpdatedAt,
    hasExistingStatus,
  ]);

  const hasContent = Boolean(text.trim() || emoji);
  const effectiveEmoji =
    emoji || (text.trim() ? DEFAULT_USER_STATUS_EMOJI : "");
  const initialEffectiveEmoji =
    baseline.emoji || (baseline.text.trim() ? DEFAULT_USER_STATUS_EMOJI : "");
  const isDirty =
    text.trim() !== baseline.text.trim() ||
    effectiveEmoji !== initialEffectiveEmoji ||
    durationTouched ||
    (duration === "Custom" &&
      baseline.duration === "Custom" &&
      baseline.customUntil !== null &&
      customUntil.getTime() !== baseline.customUntil.getTime());
  function expirationUnixSeconds(now = new Date()): number | undefined {
    if (
      !durationTouched &&
      baseline.hasExistingStatus &&
      baseline.expiresAt !== undefined
    ) {
      return baseline.expiresAt;
    }
    const expiresAt = (() => {
      switch (duration) {
        case "1 hour":
          return new Date(now.getTime() + 60 * 60_000);
        case "8 hours":
          return new Date(now.getTime() + 8 * 60 * 60_000);
        case "Today":
          return endOfToday(now);
        case "This week":
          return endOfWeek(now);
        case "Custom":
          return customUntil;
      }
    })();
    return Math.floor(expiresAt.getTime() / 1_000);
  }

  const displayedExpiration = expirationUnixSeconds();
  const expirationIsFuture =
    displayedExpiration === undefined ||
    displayedExpiration > Math.floor(Date.now() / 1_000);
  const canSave = hasContent && isDirty && expirationIsFuture;

  function handlePresetClick(preset: { text: string; emoji: string }) {
    setText(preset.text);
    setEmoji(preset.emoji);
  }

  function handleEmojiSelect(selectedEmoji: string) {
    setEmoji(selectedEmoji);
    setPickerOpen(false);
  }

  function handleSave() {
    if (!hasContent || !isDirty) return;
    const expiresAt = expirationUnixSeconds();
    if (
      expiresAt !== undefined &&
      expiresAt <= Math.floor(Date.now() / 1_000)
    ) {
      setSaveError("Choose a duration in the future.");
      return;
    }
    onSave({
      text: text.trim(),
      emoji: effectiveEmoji,
      expiresAt,
    });
    onOpenChange(false);
  }

  function handleClear() {
    onClear();
    onOpenChange(false);
  }

  function handleKeyDown(event: React.KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      handleSave();
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <ChooserDialogContent
        className="sm:max-w-[420px]"
        contentClassName="space-y-4 pt-3"
        data-testid="set-status-dialog"
        footer={
          <div className="flex w-full items-center justify-between gap-3">
            {baseline.hasExistingStatus ? (
              <Button
                className="text-destructive hover:bg-destructive/10 hover:text-destructive"
                data-testid="set-status-clear"
                onClick={handleClear}
                type="button"
                variant="ghost"
              >
                Clear status
              </Button>
            ) : (
              <span />
            )}
            <Button
              aria-label="Save status"
              data-testid="set-status-save"
              disabled={!canSave}
              onClick={handleSave}
              type="button"
            >
              Save status
            </Button>
          </div>
        }
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        headerSubtitle="Let others know what you're up to."
        title="Set a status"
      >
        <div className="flex min-h-12 items-stretch rounded-xl border border-input focus-within:ring-1 focus-within:ring-ring">
          <Popover onOpenChange={setPickerOpen} open={pickerOpen}>
            <div className="shrink-0">
              <PopoverTrigger asChild>
                <button
                  aria-label="Choose a status emoji"
                  className="flex h-12 w-12 items-center justify-center rounded-xl transition-colors hover:bg-accent"
                  type="button"
                >
                  {effectiveEmoji ? (
                    <StatusEmoji className="h-5 w-5" value={effectiveEmoji} />
                  ) : (
                    <SmilePlus className="h-5 w-5 text-muted-foreground" />
                  )}
                </button>
              </PopoverTrigger>
            </div>
            <PopoverContent
              align="start"
              className="w-auto overflow-hidden rounded-2xl p-0"
              sideOffset={4}
            >
              <EmojiPicker autoFocus onSelect={handleEmojiSelect} />
            </PopoverContent>
          </Popover>
          <input
            className="min-w-0 flex-1 bg-transparent px-1 pr-3 text-base outline-none placeholder:text-muted-foreground"
            data-testid="set-status-input"
            onChange={(event) => setText(event.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="What’s your status?"
            value={text}
          />
        </div>

        <StatusSection label="Duration">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                className={ROW_CLASS}
                data-testid="set-status-duration"
                type="button"
              >
                <Clock3 className="h-5 w-5 text-muted-foreground" />
                <span className="flex-1">Duration</span>
                <span className="text-muted-foreground">{duration}</span>
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-52">
              {DURATIONS.map((option) => (
                <DropdownMenuItem
                  className="justify-between"
                  key={option}
                  onSelect={() => {
                    setDuration(option);
                    setDurationTouched(true);
                    setSaveError("");
                  }}
                >
                  {option}
                  {duration === option ? <Check className="h-4 w-4" /> : null}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          {duration === "Custom" ? (
            <div className={ROW_CLASS}>
              <CalendarClock className="h-5 w-5 text-muted-foreground" />
              <span>Until</span>
              <Popover onOpenChange={setCalendarOpen} open={calendarOpen}>
                <PopoverTrigger asChild>
                  <button
                    aria-label="Status expiration date"
                    className="flex h-9 min-w-0 flex-1 items-center rounded-md border border-input bg-background px-2 text-sm outline-none focus:ring-1 focus:ring-ring"
                    type="button"
                  >
                    <span className="truncate">
                      {formattedDate(customUntil)}
                    </span>
                    <ChevronDown className="ml-auto h-4 w-4 shrink-0 text-muted-foreground" />
                  </button>
                </PopoverTrigger>
                <PopoverContent align="start" className="w-auto p-0">
                  <Calendar
                    disabled={{ before: startOfToday() }}
                    mode="single"
                    onSelect={(selected) => {
                      if (!selected) return;
                      setDurationTouched(true);
                      setSaveError("");
                      setCustomUntil((current) =>
                        withCustomDate(current, selected),
                      );
                      setCalendarOpen(false);
                    }}
                    selected={customUntil}
                  />
                </PopoverContent>
              </Popover>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    aria-label="Status expiration time"
                    className="flex h-9 w-28 shrink-0 items-center rounded-md border border-input bg-background px-2 text-sm outline-none focus:ring-1 focus:ring-ring"
                    type="button"
                  >
                    <span>
                      {HALF_HOUR_TIMES.find(
                        (time) => time.value === toLocalTimeValue(customUntil),
                      )?.label ?? formattedTime(customUntil)}
                    </span>
                    <ChevronDown className="ml-auto h-4 w-4 shrink-0 text-muted-foreground" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent
                  align="end"
                  className="max-h-[23rem]"
                  data-testid="status-expiration-time-menu"
                  style={{
                    minWidth: "var(--radix-dropdown-menu-trigger-width)",
                  }}
                >
                  {HALF_HOUR_TIMES.map((time) => (
                    <DropdownMenuItem
                      className="justify-between"
                      key={time.value}
                      onSelect={() => {
                        setDurationTouched(true);
                        setSaveError("");
                        setCustomUntil((current) =>
                          withCustomTime(current, time.value),
                        );
                      }}
                    >
                      {time.label}
                      {time.value === toLocalTimeValue(customUntil) ? (
                        <Check className="h-4 w-4" />
                      ) : null}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          ) : null}
          {saveError || (!expirationIsFuture && isDirty) ? (
            <p className="px-3 py-2 text-xs text-destructive" role="alert">
              {saveError || "Choose a duration in the future."}
            </p>
          ) : null}
        </StatusSection>

        {!baseline.hasExistingStatus ? (
          <StatusSection label="Quick statuses">
            {PRESETS.map((preset) => (
              <button
                className={ROW_CLASS}
                data-testid={`set-status-preset-${preset.text.toLowerCase().replace(/\s+/g, "-")}`}
                key={preset.text}
                onClick={() => handlePresetClick(preset)}
                type="button"
              >
                <span
                  aria-hidden="true"
                  className="flex w-5 justify-center text-lg"
                >
                  {preset.emoji}
                </span>
                <span>{preset.text}</span>
              </button>
            ))}
          </StatusSection>
        ) : null}
      </ChooserDialogContent>
    </Dialog>
  );
}
