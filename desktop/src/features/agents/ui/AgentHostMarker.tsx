import { cn } from "@/shared/lib/cn";
import { Cloud } from "lucide-react";
import {
  locationLabels,
  type PresenceRun,
} from "@/features/presence/runPresence";
import { OtherSetupAgentMarker } from "./OtherSetupAgentMarker";

/** Provenance never supplies location; only unexpired relay run leases do. */
export function AgentHostMarker({
  runs,
  now,
  otherSetup = false,
  className,
  testId,
}: {
  runs?: PresenceRun[];
  now: number;
  otherSetup?: boolean;
  className?: string;
  testId?: string;
}) {
  const labels = locationLabels(runs, now);
  if (!labels.length)
    return otherSetup ? (
      <OtherSetupAgentMarker className={className} testId={testId} />
    ) : null;
  const title = `Running on ${labels.join(", ")}`;
  return (
    <span
      className={cn(
        "inline-flex min-w-0 items-center gap-1 text-2xs text-muted-foreground",
        className,
      )}
      role="img"
      aria-label={title}
      title={title}
      data-testid={testId}
    >
      <Cloud aria-hidden="true" className="h-3 w-3 shrink-0" />
      <span className="max-w-32 truncate">
        {labels.length === 1 ? labels[0] : `${labels[0]} +${labels.length - 1}`}
      </span>
    </span>
  );
}
