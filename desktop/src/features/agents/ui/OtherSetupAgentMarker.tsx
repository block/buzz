import { Cloud } from "lucide-react";

import { cn } from "@/shared/lib/cn";

const OTHER_SETUP_LABEL = "From another Buzz setup";

export function OtherSetupAgentMarker({
  className,
  testId,
}: {
  className?: string;
  testId?: string;
}) {
  return (
    <span
      aria-label={OTHER_SETUP_LABEL}
      className={cn("inline-flex shrink-0", className)}
      data-testid={testId}
      role="img"
      title={OTHER_SETUP_LABEL}
    >
      <Cloud aria-hidden="true" className="h-3 w-3" />
    </span>
  );
}
