import type { BattleRhythmEvent } from "../domain/contracts";
import { getWeekRange, overlapsRange } from "../domain/dateRange";
export function WeekCalendar({
  day,
  events,
  timeZone,
  onEdit,
}: {
  day: string;
  events: readonly BattleRhythmEvent[];
  timeZone: string;
  onEdit?: (event: BattleRhythmEvent) => void;
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
          (weekday, offset) => (
            <div className="min-h-80 rounded border p-2" key={weekday}>
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
          ),
        )}
      </div>
    </div>
  );
}
