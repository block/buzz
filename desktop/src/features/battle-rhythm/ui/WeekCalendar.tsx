import type { PlanTaskCalendarProjection } from "@/features/plans/domain/calendarProjection";
import type { BattleRhythmEvent } from "../domain/contracts";
import { getWeekRange, overlapsRange } from "../domain/dateRange";
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
        {["mon", "tue", "wed", "thu", "fri", "sat", "sun"].map(
          (weekday, offset) => {
            const date = new Date(`${range.start.slice(0, 10)}T12:00:00Z`);
            date.setUTCDate(date.getUTCDate() + offset);
            const dateKey = date.toISOString().slice(0, 10);
            return (
              <div className="min-h-80 rounded border p-2" key={weekday}>
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
                          : new Intl.DateTimeFormat("en-AU", {
                              hour: "numeric",
                              minute: "2-digit",
                              timeZone,
                            }).format(new Date(event.start))}
                      </span>
                      {event.title}
                    </button>
                  ))}
              </div>
            );
          },
        )}
      </div>
    </div>
  );
}
