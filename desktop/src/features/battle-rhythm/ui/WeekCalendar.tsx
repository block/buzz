import type { PlanTaskCalendarProjection } from "@/features/plans/domain/calendarProjection";
import type { BattleRhythmEvent } from "../domain/contracts";
import { formatShipTime, weekDayHeading } from "../domain/calendarPresentation";
import {
  programEventTone,
  weekAllDayPlacement,
} from "../domain/eventPresentation";
import {
  addDays,
  getWeekRange,
  overlapsCalendarDay,
  overlapsRange,
} from "../domain/dateRange";
import { programEventClasses } from "./programEventStyles";

export function WeekCalendar({
  day,
  events,
  planMilestones,
  timeZone,
  onEdit,
  onOpenPlanMilestone,
}: {
  day: string;
  events: readonly BattleRhythmEvent[];
  planMilestones: readonly PlanTaskCalendarProjection[];
  timeZone: string;
  onEdit?: (event: BattleRhythmEvent) => void;
  onOpenPlanMilestone?: (milestone: PlanTaskCalendarProjection) => void;
}) {
  const range = getWeekRange(day, timeZone);
  const shown = events.filter((event) =>
    overlapsRange(event.start, event.end, range),
  );
  const allDayEvents = shown.filter((event) => event.allDay);
  const timedEvents = shown.filter((event) => !event.allDay);
  return (
    <div>
      <div
        className="mb-2 rounded border border-dashed p-2"
        data-testid="week-all-day-lane"
      >
        <div className="mb-2 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
          All-day activities
        </div>
        <div className="grid gap-1">
          {allDayEvents.map((event) => {
            const placement = weekAllDayPlacement(event, range, timeZone);
            if (!placement) return null;
            return (
              <div className="grid grid-cols-7 gap-2" key={event.id}>
                <button
                  aria-label={`All day ${event.title}`}
                  className={`w-full rounded border px-2 py-1 text-left text-xs ${programEventClasses(event)}`}
                  data-program-tone={programEventTone(event)}
                  onClick={() => onEdit?.(event)}
                  style={{
                    gridColumn: `${placement.startColumn} / span ${placement.span}`,
                  }}
                  title={
                    event.location
                      ? `${event.title} · ${event.location}`
                      : event.title
                  }
                  type="button"
                >
                  {event.title}
                </button>
              </div>
            );
          })}
        </div>
      </div>
      <div className="grid grid-cols-7 gap-2" data-testid="week-timed-columns">
        {Array.from({ length: 7 }, (_, offset) => {
          const dateKey = addDays(range.start.slice(0, 10), offset);
          return (
            <div
              className="min-h-80 overflow-hidden rounded border bg-card/30"
              key={dateKey}
            >
              <div className="border-b bg-muted/40 px-2 py-2 text-center text-xs font-semibold tracking-wide">
                {weekDayHeading(dateKey, timeZone)}
              </div>
              <div className="p-2">
                {planMilestones
                  .filter((milestone) => milestone.date === dateKey)
                  .map((milestone) => (
                    <button
                      className="mb-2 w-full rounded border border-amber-500/40 bg-amber-500/10 p-2 text-left text-xs text-amber-700 dark:text-amber-300"
                      data-testid="plan-task-milestone"
                      key={milestone.id}
                      onClick={() => onOpenPlanMilestone?.(milestone)}
                      type="button"
                    >
                      <span className="block text-2xs uppercase tracking-wide">
                        Plan milestone
                      </span>
                      {milestone.title}
                    </button>
                  ))}
                {timedEvents
                  .filter((event) =>
                    overlapsCalendarDay(
                      event.start,
                      event.end,
                      dateKey,
                      timeZone,
                    ),
                  )
                  .map((event) => (
                    <button
                      className={`mb-2 rounded border p-2 text-xs ${programEventClasses(event)}`}
                      data-program-tone={programEventTone(event)}
                      key={event.id}
                      onClick={() => onEdit?.(event)}
                      type="button"
                    >
                      <span className="block text-2xs text-muted-foreground">
                        {event.allDay
                          ? "All day"
                          : formatShipTime(event.start, timeZone)}
                      </span>
                      {event.title}
                    </button>
                  ))}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
