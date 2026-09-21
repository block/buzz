import { AlertTriangle } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { i18n, useTranslation } from "@/i18n";
import { Input } from "@/shared/ui/input";
import { CronExpressionInput } from "./CronExpressionInput";
import { FieldLabel, FormSelect } from "./workflowFormPrimitives";
import type { TriggerConfig } from "./workflowFormTypes";
import {
  SCHEDULE_FREQUENCIES,
  scheduleFormFromTrigger,
  scheduleFrequencyLabel,
  scheduleTriggerFromForm,
  scheduleWeekdaysFromCronField,
} from "./workflowSchedule";
import type { ScheduleFormState } from "./workflowSchedule";

type WeekdayId =
  | "friday"
  | "monday"
  | "saturday"
  | "sunday"
  | "thursday"
  | "tuesday"
  | "wednesday";

/** `short` is the single-cell initial; it is localized separately. */
const WEEKDAYS: Array<{ short: string; value: string; weekday: WeekdayId }> = [
  { value: "0", weekday: "sunday", short: "S" },
  { value: "1", weekday: "monday", short: "M" },
  { value: "2", weekday: "tuesday", short: "T" },
  { value: "3", weekday: "wednesday", short: "W" },
  { value: "4", weekday: "thursday", short: "T" },
  { value: "5", weekday: "friday", short: "F" },
  { value: "6", weekday: "saturday", short: "S" },
];

function weekdayLabel(weekday: WeekdayId): string {
  switch (weekday) {
    case "friday":
      return i18n.t("workflows.schedule.weekday-friday");
    case "monday":
      return i18n.t("workflows.schedule.weekday-monday");
    case "saturday":
      return i18n.t("workflows.schedule.weekday-saturday");
    case "sunday":
      return i18n.t("workflows.schedule.weekday-sunday");
    case "thursday":
      return i18n.t("workflows.schedule.weekday-thursday");
    case "tuesday":
      return i18n.t("workflows.schedule.weekday-tuesday");
    case "wednesday":
      return i18n.t("workflows.schedule.weekday-wednesday");
  }
}

function weekdayInitial(weekday: WeekdayId): string {
  switch (weekday) {
    case "friday":
      return i18n.t("workflows.schedule.weekday-initial-friday");
    case "monday":
      return i18n.t("workflows.schedule.weekday-initial-monday");
    case "saturday":
      return i18n.t("workflows.schedule.weekday-initial-saturday");
    case "sunday":
      return i18n.t("workflows.schedule.weekday-initial-sunday");
    case "thursday":
      return i18n.t("workflows.schedule.weekday-initial-thursday");
    case "tuesday":
      return i18n.t("workflows.schedule.weekday-initial-tuesday");
    case "wednesday":
      return i18n.t("workflows.schedule.weekday-initial-wednesday");
  }
}

const MONTH_DAYS = Array.from({ length: 31 }, (_, index) => String(index + 1));

function monthlyDayWarning(monthDay: string): string | null {
  return Number(monthDay) > 28
    ? i18n.t("workflows.schedule.monthly-day-warning")
    : null;
}

function customCronSeed(schedule: ScheduleFormState): string {
  const currentTrigger = scheduleTriggerFromForm(schedule);
  if (currentTrigger.cron) return currentTrigger.cron;

  switch (currentTrigger.interval) {
    case "15m":
      return "*/15 * * * *";
    case "30m":
      return "*/30 * * * *";
    case "1h":
      return "0 * * * *";
    default:
      return "";
  }
}

export function WorkflowScheduleFields({
  disabled,
  onUpdate,
  trigger,
}: {
  disabled?: boolean;
  onUpdate: (trigger: TriggerConfig) => void;
  trigger: TriggerConfig;
}) {
  const { t } = useTranslation();
  const [forceCustomCron, setForceCustomCron] = React.useState(false);
  const parsedSchedule = scheduleFormFromTrigger(trigger);
  const schedule: ScheduleFormState = forceCustomCron
    ? {
        ...parsedSchedule,
        customCron: trigger.cron ?? parsedSchedule.customCron,
        frequency: "custom_cron",
      }
    : parsedSchedule;
  const updateSchedule = (updates: Partial<ScheduleFormState>) => {
    onUpdate(scheduleTriggerFromForm({ ...schedule, ...updates }));
  };
  const usesTime = ["daily", "weekly", "monthly"].includes(schedule.frequency);
  const selectedWeekdays = new Set(
    scheduleWeekdaysFromCronField(schedule.weekday),
  );
  const warning = monthlyDayWarning(schedule.monthDay);

  return (
    <div className="space-y-3">
      <fieldset>
        <legend className="sr-only">{t("workflows.schedule.repeats")}</legend>
        <div className="grid grid-cols-2 gap-2.5">
          {SCHEDULE_FREQUENCIES.map((frequency) => {
            const id = `wf-trigger-frequency-${frequency}`;
            return (
              <div
                className={cn(
                  "relative",
                  frequency === "custom_cron" && "col-span-2",
                )}
                key={frequency}
              >
                <input
                  checked={schedule.frequency === frequency}
                  className="peer sr-only"
                  disabled={disabled}
                  id={id}
                  name="wf-trigger-frequency"
                  onChange={() => {
                    const isCustom = frequency === "custom_cron";
                    setForceCustomCron(isCustom);
                    updateSchedule({
                      customCron: isCustom
                        ? customCronSeed(schedule)
                        : schedule.customCron,
                      frequency,
                    });
                  }}
                  type="radio"
                  value={frequency}
                />
                <label
                  className={cn(
                    "flex min-h-12 cursor-pointer items-center justify-center rounded-lg border px-3 py-2 text-center text-sm font-medium",
                    "outline-2 outline-offset-2 outline-transparent transition-[background-color,border-color,color,outline-color]",
                    "peer-focus-visible:ring-2 peer-focus-visible:ring-ring",
                    "peer-disabled:cursor-not-allowed peer-disabled:opacity-50",
                    schedule.frequency === frequency
                      ? "border-border/0 bg-transparent text-foreground outline-foreground/45"
                      : "border-border/70 bg-background/35 text-muted-foreground hover:border-border hover:bg-muted/55 hover:text-foreground hover:outline-muted-foreground/20",
                  )}
                  htmlFor={id}
                >
                  {scheduleFrequencyLabel(frequency)}
                </label>
              </div>
            );
          })}
        </div>
      </fieldset>

      {schedule.frequency === "weekly" ? (
        <fieldset className="space-y-1.5">
          <legend className="text-xs font-medium text-muted-foreground">
            {t("workflows.schedule.repeat-on")}
          </legend>
          <div className="grid grid-cols-7 gap-2">
            {WEEKDAYS.map(({ value, weekday }) => {
              const id = `wf-trigger-weekday-${value}`;
              return (
                <div className="relative" key={value}>
                  <input
                    aria-label={weekdayLabel(weekday)}
                    checked={selectedWeekdays.has(value)}
                    className="peer sr-only"
                    disabled={disabled}
                    id={id}
                    name="wf-trigger-weekday"
                    onChange={() => {
                      const nextWeekdays = new Set(selectedWeekdays);
                      if (nextWeekdays.has(value)) {
                        if (nextWeekdays.size === 1) return;
                        nextWeekdays.delete(value);
                      } else {
                        nextWeekdays.add(value);
                      }
                      updateSchedule({
                        weekday: WEEKDAYS.map(({ value: day }) => day)
                          .filter((day) => nextWeekdays.has(day))
                          .join(","),
                      });
                    }}
                    type="checkbox"
                    value={value}
                  />
                  <label
                    className={cn(
                      "flex aspect-square cursor-pointer items-center justify-center rounded-full text-xs font-medium transition-colors",
                      "peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-2",
                      "peer-disabled:cursor-not-allowed peer-disabled:opacity-50",
                      selectedWeekdays.has(value)
                        ? "bg-primary text-primary-foreground"
                        : "bg-muted text-muted-foreground hover:bg-muted/70 hover:text-foreground",
                    )}
                    htmlFor={id}
                  >
                    {weekdayInitial(weekday)}
                  </label>
                </div>
              );
            })}
          </div>
        </fieldset>
      ) : null}

      {schedule.frequency === "monthly" ? (
        <div className="space-y-1.5">
          <FieldLabel htmlFor="wf-trigger-month-day">
            {t("workflows.schedule.day-of-month")}
          </FieldLabel>
          <FormSelect
            disabled={disabled}
            id="wf-trigger-month-day"
            onChange={(monthDay) => updateSchedule({ monthDay })}
            value={schedule.monthDay}
          >
            {MONTH_DAYS.map((day) => (
              <option key={day} value={day}>
                {day}
              </option>
            ))}
          </FormSelect>
          {warning ? (
            <div
              className="flex gap-2 rounded-lg border border-warning/30 bg-warning-bg px-3 py-2"
              role="status"
            >
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
              <p className="text-xs leading-5 text-warning">{warning}</p>
            </div>
          ) : null}
        </div>
      ) : null}

      {usesTime ? (
        <div className="space-y-1.5">
          <FieldLabel htmlFor="wf-trigger-time">
            {t("workflows.schedule.run-time")}
          </FieldLabel>
          <Input
            disabled={disabled}
            id="wf-trigger-time"
            onChange={(event) => updateSchedule({ time: event.target.value })}
            type="time"
            value={schedule.time}
          />
        </div>
      ) : null}

      {schedule.frequency === "custom_cron" ? (
        <CronExpressionInput
          disabled={disabled}
          onChange={(customCron) => updateSchedule({ customCron })}
          value={schedule.customCron}
        />
      ) : null}

      {schedule.frequency === "custom_interval" ? (
        <div className="space-y-1.5">
          <FieldLabel htmlFor="wf-trigger-interval">
            {t("workflows.schedule.existing-interval")}
          </FieldLabel>
          <Input
            autoCapitalize="off"
            autoCorrect="off"
            disabled={disabled}
            id="wf-trigger-interval"
            onChange={(event) =>
              updateSchedule({ customInterval: event.target.value })
            }
            value={schedule.customInterval}
          />
          <p className="text-xs text-muted-foreground">
            {t("workflows.schedule.interval-note")}
          </p>
        </div>
      ) : null}
    </div>
  );
}
