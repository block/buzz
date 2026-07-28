import type { BattleRhythmEvent } from "../domain/contracts";
export function DayShortcast({
  events,
  routineState,
  timeZone,
  onEdit,
}: {
  events: readonly BattleRhythmEvent[];
  routineState: string;
  timeZone: string;
  onEdit?: (event: BattleRhythmEvent) => void;
}) {
  return (
    <div>
      <p className="mb-3 text-sm text-muted-foreground">
        Routine state:{" "}
        <span className="font-medium text-foreground">{routineState}</span>
      </p>
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
                    : new Intl.DateTimeFormat("en-AU", {
                        hour: "numeric",
                        minute: "2-digit",
                        timeZone,
                      }).format(new Date(event.start))}
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
