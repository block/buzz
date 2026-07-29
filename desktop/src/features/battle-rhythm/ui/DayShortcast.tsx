import type { PlanTaskCalendarProjection } from "@/features/plans/domain/calendarProjection";
import type { BattleRhythmEvent } from "../domain/contracts";
import { formatShipTime } from "../domain/calendarPresentation";

export function DayShortcast({
  events,
  planMilestones,
  routineState,
  timeZone,
  onEdit,
  onOpenPlanMilestone,
}: {
  events: readonly BattleRhythmEvent[];
  planMilestones: readonly PlanTaskCalendarProjection[];
  routineState: string;
  timeZone: string;
  onEdit?: (event: BattleRhythmEvent) => void;
  onOpenPlanMilestone?: (milestone: PlanTaskCalendarProjection) => void;
}) {
  return (
    <div>
      <p className="mb-3 text-sm text-muted-foreground">
        Routine state:{" "}
        <span className="font-medium text-foreground">{routineState}</span>
      </p>
      {planMilestones.length ? (
        <div className="mb-3 grid gap-2">
          {planMilestones.map((milestone) => (
            <button
              className="rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-left text-sm"
              data-testid="plan-task-milestone"
              key={milestone.id}
              onClick={() => onOpenPlanMilestone?.(milestone)}
              type="button"
            >
              <span className="mr-2 text-2xs uppercase tracking-wide text-amber-700 dark:text-amber-300">
                Plan milestone
              </span>
              {milestone.title}
            </button>
          ))}
        </div>
      ) : null}
      <div className="overflow-x-auto">
        <table className="w-full text-left text-sm">
          <thead className="border-b text-2xs uppercase tracking-wide text-muted-foreground">
            <tr>
              <th>Time</th>
              <th>Event</th>
              <th>I/C</th>
              <th>Remarks</th>
            </tr>
          </thead>
          <tbody>
            {events.map((event) => (
              <tr className="border-b" key={event.id}>
                <td className="py-2">
                  {event.allDay
                    ? "All day"
                    : formatShipTime(event.start, timeZone)}
                </td>
                <td>
                  <button onClick={() => onEdit?.(event)} type="button">
                    {event.title}
                  </button>
                  <span className="ml-2 text-2xs text-muted-foreground">
                    {event.ownership.kind === "manual" ? "Manual" : "Source"}
                  </span>
                </td>
                <td>{event.responsibleOwner ?? "—"}</td>
                <td>{event.remarks ?? "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
