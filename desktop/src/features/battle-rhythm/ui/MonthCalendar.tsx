import type { BattleRhythmEvent } from "../domain/contracts";
import { getMonthCells } from "../domain/dateRange";
export function MonthCalendar({
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
  const cells = getMonthCells(day, timeZone);
  return (
    <div className="grid grid-cols-7 gap-px overflow-hidden rounded border bg-border">
      {cells.map((cell) => (
        <div className="min-h-28 bg-background p-2" key={cell}>
          <div className="mb-1 text-2xs text-muted-foreground">
            {cell.slice(-2)}
          </div>
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
