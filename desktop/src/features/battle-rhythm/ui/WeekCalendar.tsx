import type { PlanTaskCalendarProjection } from "@/features/plans/domain/calendarProjection";
import type { BattleRhythmEvent } from "../domain/contracts";
import { formatShipTime, weekDayHeading } from "../domain/calendarPresentation";
import { getWeekRange, overlapsRange } from "../domain/dateRange";

function addDays(day: string, amount: number): string {
  const date = new Date(`${day}T12:00:00Z`);
  date.setUTCDate(date.getUTCDate() + amount);
  return date.toISOString().slice(0, 10);
}

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
  return (
    <div>
      <div className="mb-2 rounded border border-dashed p-2 text-2xs text-muted-foreground">
        All-day activities
      </div>
      <div className="grid grid-cols-7 gap-2">
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
                {shown
                  .filter(
                    (event) =>
                      new Date(event.start).getDay() === (offset + 1) % 7,
                  )
                  .map((event) => (
                    <button
                      className="mb-2 rounded bg-primary/10 p-2 text-xs"
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
