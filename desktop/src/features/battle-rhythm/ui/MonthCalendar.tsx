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
    <div className="grid grid-cols-7 gap-px overflow-hidden rounded border bg-border">
      {cells.map((cell) => (
        <div className="min-h-28 bg-background p-2" key={cell}>
          <div className="mb-1 text-2xs text-muted-foreground">
            {cell.slice(-2)}
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
      ))}
    </div>
  );
}
