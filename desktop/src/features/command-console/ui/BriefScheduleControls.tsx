import type { BriefSchedule } from "@/features/command-console/domain/briefContracts";
import type { CommandBriefScheduleUpdate } from "@/shared/api/tauriCommandBrief";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";

export function BriefScheduleControls({
  schedule,
  disabled,
  onChange,
}: {
  schedule: BriefSchedule;
  disabled: boolean;
  onChange: (update: CommandBriefScheduleUpdate) => void;
}) {
  const update = (patch: Partial<CommandBriefScheduleUpdate>) =>
    onChange({
      enabled: schedule.enabled,
      localTime: schedule.localTime,
      concurrency: schedule.concurrency,
      ...patch,
    });

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Daily schedule</CardTitle>
      </CardHeader>
      <CardContent className="grid gap-4 sm:grid-cols-3">
        <label
          className="flex items-center gap-3 text-sm"
          htmlFor="daily-command-brief-enabled"
        >
          <Switch
            aria-label="Enable scheduled Daily Command Brief"
            checked={schedule.enabled}
            disabled={disabled}
            id="daily-command-brief-enabled"
            onCheckedChange={(checked) => update({ enabled: checked })}
          />
          <span>Enabled</span>
        </label>
        <label className="space-y-1 text-sm" htmlFor="daily-command-brief-time">
          <span className="font-medium">Local time</span>
          <Input
            aria-label="Daily Command Brief local time"
            disabled={disabled}
            id="daily-command-brief-time"
            onChange={(event) => update({ localTime: event.target.value })}
            type="time"
            value={schedule.localTime}
          />
        </label>
        <label
          className="space-y-1 text-sm"
          htmlFor="daily-command-brief-concurrency"
        >
          <span className="font-medium">Local model capacity</span>
          <select
            aria-label="Local model concurrency"
            className="h-9 w-full rounded-lg border border-input/40 bg-background px-3 text-sm focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
            disabled={disabled}
            id="daily-command-brief-concurrency"
            onChange={(event) =>
              update({ concurrency: Number(event.target.value) as 1 | 2 })
            }
            value={schedule.concurrency}
          >
            <option value={1}>1 adviser at a time</option>
            <option value={2}>2 advisers at a time</option>
          </select>
        </label>
        <p className="text-sm text-muted-foreground sm:col-span-3">
          Timezone: {schedule.timezone}. macOS may delay generation while the
          Mac sleeps; same-day catch-up runs after wake when enabled.
        </p>
      </CardContent>
    </Card>
  );
}
