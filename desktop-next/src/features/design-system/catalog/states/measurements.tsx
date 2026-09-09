import { Meter, Progress } from "@buzz/ui";

/** Empty, partial, and complete values rendered by the actual progress API. */
export function ProgressStates() {
  return (
    <div className="component-state-grid">
      {[0, 50, 100].map((value) => (
        <Progress.Root key={value} value={value}>
          <div className="bui-inline">
            <Progress.Label>
              {value === 0
                ? "Not started"
                : value === 100
                  ? "Complete"
                  : "In progress"}
            </Progress.Label>
            <Progress.Value />
          </div>
          <Progress.Track>
            <Progress.Indicator />
          </Progress.Track>
        </Progress.Root>
      ))}
    </div>
  );
}

/** A meter measures a bounded quantity independently of task completion. */
export function MeterStates() {
  return (
    <div className="component-state-grid">
      {[10, 50, 100].map((value) => (
        <Meter.Root key={value} value={value}>
          <div className="bui-inline">
            <Meter.Label>
              {value === 10 ? "Low usage" : value === 50 ? "Half used" : "Full"}
            </Meter.Label>
            <Meter.Value />
          </div>
          <Meter.Track>
            <Meter.Indicator />
          </Meter.Track>
        </Meter.Root>
      ))}
    </div>
  );
}
