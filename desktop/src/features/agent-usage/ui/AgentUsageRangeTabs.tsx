import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import {
  buildLocalDayBoundaries,
  formatLocalDate,
  validateCustomRange,
  type UsageRange,
} from "../lib/agentUsage";

const PRESET_DAYS = [1, 7, 30] as const;

type PresetDays = (typeof PRESET_DAYS)[number];

/**
 * Window selector shared by the Agents-overview and per-agent usage views:
 * the `1d`/`7d`/`30d` presets plus a `Custom` tab whose popover takes an
 * arbitrary inclusive start/end date pair.
 *
 * Selecting `Custom` opens the picker without changing the active range —
 * the range only moves once a valid pair is applied, so an in-progress edit
 * never issues a query. Validation is local ({@link validateCustomRange}),
 * so the user sees "pick a range of 366 days or fewer" rather than the
 * backend's fail-closed arity error.
 *
 * `testIdPrefix` keeps the two mounted instances addressable independently
 * (`agent-usage-window-*` on the overview, `agent-usage-focused-window-*` in
 * the profile panel).
 */
export function AgentUsageRangeTabs({
  onRangeChange,
  range,
  testIdPrefix,
}: {
  onRangeChange: (range: UsageRange) => void;
  range: UsageRange;
  testIdPrefix: string;
}) {
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const customTriggerRef = React.useRef<HTMLButtonElement>(null);

  return (
    // `PopoverAnchor`, not `PopoverTrigger`: the trigger would spread its own
    // `data-state` ("open"/"closed") onto the tab and clobber the tab's
    // "active"/"inactive" state. The anchor only positions, so the picker
    // opens from an explicit click and the tab keeps its own state.
    <Popover onOpenChange={setPickerOpen} open={pickerOpen}>
      <Tabs
        onValueChange={(value) => {
          const days = Number(value);
          if (isPresetDays(days)) onRangeChange({ kind: "preset", days });
        }}
        value={range.kind === "preset" ? String(range.days) : "custom"}
      >
        <TabsList>
          {PRESET_DAYS.map((days) => (
            <TabsTrigger
              data-testid={`${testIdPrefix}-${days}`}
              key={days}
              value={String(days)}
            >
              {days}d
            </TabsTrigger>
          ))}
          <PopoverAnchor asChild>
            <TabsTrigger
              data-testid={`${testIdPrefix}-custom`}
              onClick={() => setPickerOpen(true)}
              ref={customTriggerRef}
              value="custom"
            >
              Custom
            </TabsTrigger>
          </PopoverAnchor>
        </TabsList>
      </Tabs>

      <PopoverContent
        align="end"
        className="space-y-3"
        data-testid={`${testIdPrefix}-custom-popover`}
        onCloseAutoFocus={(event) => {
          // No `PopoverTrigger` to return focus to, so restore it manually.
          event.preventDefault();
          customTriggerRef.current?.focus();
        }}
      >
        <CustomRangeForm
          onApply={(applied) => {
            onRangeChange(applied);
            setPickerOpen(false);
          }}
          range={range}
          testIdPrefix={testIdPrefix}
        />
      </PopoverContent>
    </Popover>
  );
}

function CustomRangeForm({
  onApply,
  range,
  testIdPrefix,
}: {
  onApply: (range: UsageRange) => void;
  range: UsageRange;
  testIdPrefix: string;
}) {
  // Seeded once per mount; Radix unmounts the popover's content when it
  // closes, so each open re-seeds from the range that is active then.
  const [draft, setDraft] = React.useState(() => initialDraft(range));
  const { startDate, endDate } = draft;

  const validation = validateCustomRange(startDate, endDate);
  const today = formatLocalDate(new Date());

  return (
    <form
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (validation.ok) onApply({ kind: "custom", startDate, endDate });
      }}
    >
      <p className="text-sm font-medium text-foreground">Custom range</p>
      <div className="flex items-center gap-2">
        <Input
          aria-label="Usage range start date"
          data-testid={`${testIdPrefix}-custom-start`}
          max={today}
          onChange={(event) =>
            setDraft((current) => ({
              ...current,
              startDate: event.target.value,
            }))
          }
          type="date"
          value={startDate}
        />
        <span className="text-sm text-muted-foreground">to</span>
        <Input
          aria-label="Usage range end date"
          data-testid={`${testIdPrefix}-custom-end`}
          max={today}
          onChange={(event) =>
            setDraft((current) => ({ ...current, endDate: event.target.value }))
          }
          type="date"
          value={endDate}
        />
      </div>
      {validation.ok ? (
        <p
          className="text-xs text-muted-foreground"
          data-testid={`${testIdPrefix}-custom-summary`}
        >
          {validation.days} day{validation.days === 1 ? "" : "s"} selected.
        </p>
      ) : (
        <p
          className="text-xs text-destructive"
          data-testid={`${testIdPrefix}-custom-error`}
        >
          {validation.message}
        </p>
      )}
      <Button
        className="w-full"
        data-testid={`${testIdPrefix}-custom-apply`}
        disabled={!validation.ok}
        size="sm"
        type="submit"
      >
        Apply
      </Button>
    </form>
  );
}

function isPresetDays(days: number): days is PresetDays {
  return (PRESET_DAYS as readonly number[]).includes(days);
}

/**
 * Pre-fill for the picker: the active custom range when there is one,
 * otherwise the span the active preset already covers, so applying without
 * edits is a no-op rather than an empty form. Endpoints come from
 * {@link buildLocalDayBoundaries} so the pre-filled dates are exactly the
 * civil days the preset queried.
 */
function initialDraft(range: UsageRange): {
  startDate: string;
  endDate: string;
} {
  if (range.kind === "custom") {
    return { startDate: range.startDate, endDate: range.endDate };
  }
  const boundaries = buildLocalDayBoundaries(range.days);
  // Boundaries close the final day, so the last covered day starts at the
  // second-to-last boundary.
  const first = boundaries[0] ?? 0;
  const lastDayStart = boundaries[boundaries.length - 2] ?? first;
  return {
    startDate: formatLocalDate(new Date(first * 1_000)),
    endDate: formatLocalDate(new Date(lastDayStart * 1_000)),
  };
}
