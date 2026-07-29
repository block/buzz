import type { PlanTaskCalendarProjection } from "@/features/plans/domain/calendarProjection";
import type { BattleRhythmEvent } from "../domain/contracts";
import { getMonthCells } from "../domain/dateRange";
export function MonthCalendar({
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
  const cells = getMonthCells(day, timeZone);
  return (
    <div className="overflow-hidden rounded border bg-border">
      <div className="grid grid-cols-7 gap-px">
        {[
          "Monday",
          "Tuesday",
          "Wednesday",
          "Thursday",
          "Friday",
          "Saturday",
          "Sunday",
        ].map((weekday) => (
          <div
            className="bg-muted/50 px-2 py-2 text-center text-xs font-medium text-muted-foreground"
            key={weekday}
          >
            {weekday}
          </div>
        ))}
      </div>
      <div className="grid grid-cols-7 gap-px">
        {cells.map((cell) => {
          const inMonth = cell.slice(0, 7) === day.slice(0, 7);
          return (
            <div
              className={`min-h-28 bg-background p-2 ${inMonth ? "" : "opacity-45"}`}
              key={cell}
            >
              <div className="mb-1 text-sm font-medium text-muted-foreground">
                {Number(cell.slice(-2))}
              </div>
              {planMilestones
                .filter((milestone) => milestone.date === cell)
                .map((milestone) => (
                  <button
                    className="mb-1 block w-full truncate rounded border border-amber-500/40 bg-amber-500/10 px-1 text-left text-2xs text-amber-700 dark:text-amber-300"
                    data-testid="plan-task-milestone"
                    key={milestone.id}
                    onClick={() => onOpenPlanMilestone?.(milestone)}
                    title={`${milestone.title} · Plan milestone`}
                    type="button"
                  >
                    ◆ {milestone.title}
                  </button>
                ))}
              {events
                .filter((event) => event.start.slice(0, 10) === cell)
                .map((event) => (
                  <button
                    className="block w-full truncate rounded bg-primary/10 px-1 text-left text-2xs text-primary"
                    key={event.id}
                    onClick={() => onEdit?.(event)}
                    title={event.title}
                    type="button"
                  >
                    {event.title}
                  </button>
                ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
