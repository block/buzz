import type { ObserverEvent } from "./agentSessionTypes";
import { describeRawEvent } from "./agentSessionTranscript";
import { TranscriptTimestamp } from "./activityRenderClasses/TranscriptTimestamp";
import { useTranscriptTimestampsEnabled } from "./transcriptTimestampPreference";

export function RawEventRail({ events }: { events: ObserverEvent[] }) {
  const showTimestamps = useTranscriptTimestampsEnabled();

  return (
    <section className="flex min-h-0 w-full flex-col text-foreground">
      <div className="min-h-0 flex-1">
        {events.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            No raw events yet.
          </p>
        ) : (
          <div className="space-y-2">
            {events.map((event) => (
              <details
                className="group rounded-md border border-border/55 bg-muted/25 px-2.5 py-1.5 transition-colors open:bg-muted/35"
                key={event.seq}
              >
                <summary className="cursor-pointer select-none text-xs text-muted-foreground transition-colors group-open:text-foreground">
                  <span className="font-mono text-muted-foreground/70">
                    #{event.seq}
                  </span>{" "}
                  {describeRawEvent(event)}
                  {showTimestamps ? (
                    <span
                      className="mt-1 flex justify-start"
                      data-testid="raw-event-timestamp"
                    >
                      <TranscriptTimestamp timestamp={event.timestamp} />
                    </span>
                  ) : null}
                </summary>
                <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap wrap-break-word rounded-md border border-border/40 bg-background/45 p-2 font-mono text-xs leading-5 text-muted-foreground">
                  {JSON.stringify(event.payload, null, 2)}
                </pre>
              </details>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
