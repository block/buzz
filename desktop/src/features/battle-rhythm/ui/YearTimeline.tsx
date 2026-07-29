import type { BattleRhythmEvent } from "../domain/contracts";
import { monthGrid } from "../domain/calendarPresentation";

export function YearTimeline({
  day,
  events,
  timeZone,
}: {
  day: string;
  events: readonly BattleRhythmEvent[];
  timeZone: string;
}) {
  const months = monthGrid(day, timeZone);
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
      {months.map((month) => (
        <section className="rounded-xl border bg-card/30 p-3" key={month.month}>
          <h2 className="mb-3 text-base font-semibold">{month.label}</h2>
          <div className="mb-1 grid grid-cols-7 text-center text-2xs font-medium text-muted-foreground">
            {[
              ["mon", "M"],
              ["tue", "T"],
              ["wed", "W"],
              ["thu", "T"],
              ["fri", "F"],
              ["sat", "S"],
              ["sun", "S"],
            ].map(([key, label]) => (
              <span key={key}>{label}</span>
            ))}
          </div>
          <div className="grid grid-cols-7 gap-y-1 text-center text-xs">
            {month.cells.map((cell) => {
              const hasEvents = events.some(
                (event) => event.start.slice(0, 10) === cell,
              );
              return (
                <span
                  className={`relative rounded py-1 ${
                    cell.slice(5, 7) === String(month.month).padStart(2, "0")
                      ? "text-foreground"
                      : "text-muted-foreground/40"
                  } ${hasEvents ? "bg-primary/15 font-semibold text-primary" : ""}`}
                  key={cell}
                  title={
                    hasEvents
                      ? events
                          .filter((event) => event.start.slice(0, 10) === cell)
                          .map((event) => event.title)
                          .join(", ")
                      : undefined
                  }
                >
                  {Number(cell.slice(-2))}
                </span>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
