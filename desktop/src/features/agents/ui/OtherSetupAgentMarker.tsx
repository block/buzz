import { Cloud } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";

const OTHER_SETUP_LABEL = "From another Buzz setup";

export function OtherSetupAgentMarker({
  className,
  testId,
}: {
  className?: string;
  testId?: string;
}) {
  return (
    <Badge
      aria-label={OTHER_SETUP_LABEL}
      className={cn(
        "gap-1 px-1.5 py-0.5 font-medium normal-case tracking-normal",
        className,
      )}
      data-testid={testId}
      title={OTHER_SETUP_LABEL}
      variant="secondary"
    >
      <Cloud aria-hidden="true" className="h-3 w-3" />
      Other setup
    </Badge>
  );
}
