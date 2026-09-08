import { AlertTriangle } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { CronExpressionInput } from "./CronExpressionInput";
import { FieldLabel, FormSelect } from "./workflowFormPrimitives";
import type { TriggerConfig } from "./workflowFormTypes";
import {
  SCHEDULE_FREQUENCIES,
  SCHEDULE_FREQUENCY_LABELS,
  scheduleFormFromTrigger,
  scheduleTriggerFromForm,
  scheduleWeekdaysFromCronField,
} from "./workflowSchedule";
import type { ScheduleFormState } from "./workflowSchedule";

const WEEKDAYS = [
  ["0", "Sunday", "S"],
  ["1", "Monday", "M"],
  ["2", "Tuesday", "T"],
  ["3", "Wednesday", "W"],
  ["4", "Thursday", "T"],
  ["5", "Friday", "F"],
  ["6", "Saturday", "S"],
] as const;

const MONTH_DAYS = Array.from({ length: 31 }, (_, index) => String(index + 1));

function monthlyDayWarning(monthDay: string): string | null {
  return Number(monthDay) > 28
    ? "This schedule won’t run in some months."
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
        <legend className="sr-only">Repeats</legend>
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
                  {SCHEDULE_FREQUENCY_LABELS[frequency]}
                </label>
              </div>
            );
          })}
        </div>
      </fieldset>

      {schedule.frequency === "weekly" ? (
        <fieldset className="space-y-1.5">
          <legend className="text-xs font-medium text-muted-foreground">
            Repeat on
          </legend>
          <div className="grid grid-cols-7 gap-2">
            {WEEKDAYS.map(([value, label, shortLabel]) => {
              const id = `wf-trigger-weekday-${value}`;
              return (
                <div className="relative" key={value}>
                  <input
                    aria-label={label}
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
                        weekday: WEEKDAYS.map(([day]) => day)
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
                    {shortLabel}
                  </label>
                </div>
              );
            })}
          </div>
        </fieldset>
      ) : null}

      {schedule.frequency === "monthly" ? (
        <div className="space-y-1.5">
          <FieldLabel htmlFor="wf-trigger-month-day">Day of month</FieldLabel>
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
          <FieldLabel htmlFor="wf-trigger-time">Run time (UTC)</FieldLabel>
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
            Existing interval
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
            Keep this legacy interval or choose a repeat option above. All
            schedules use UTC.
          </p>
        </div>
      ) : null}
    </div>
  );
}
