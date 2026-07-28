import type { BattleRhythmEvent } from "../domain/contracts";
export function YearTimeline({
  events,
}: {
  events: readonly BattleRhythmEvent[];
}) {
  return (
    <div className="grid gap-2">
      {events.length ? (
        events.map((event) => (
          <div
            className="rounded border border-primary/20 bg-primary/5 px-3 py-2 text-sm"
            key={event.id}
          >
            <span className="mr-2 text-2xs uppercase tracking-wide text-muted-foreground">
              {new Intl.DateTimeFormat("en-AU", {
                month: "short",
                timeZone: event.timeZone,
              }).format(new Date(event.start))}
            </span>
            {event.title}
          </div>
        ))
      ) : (
        <p className="text-sm text-muted-foreground">
          No activities in this period.
        </p>
      )}
    </div>
  );
}
