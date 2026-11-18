import { AlertCircle } from "lucide-react";

import { useRetryBootWarm } from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";

/**
 * Inline error affordance shown when the launch runtime-catalog warm failed
 * (the boot-warm gate's `failed` state). Unlike a global-config load failure —
 * which is not retryable and keeps the "restart the app" copy — a failed
 * harness probe re-runs in place via `useRetryBootWarm`, so the create/edit
 * picker and Agent defaults surfaces both render this instead of a dead end.
 */
export function HarnessCatalogRetryNotice() {
  const retryBootWarm = useRetryBootWarm();
  return (
    <div className="flex flex-wrap items-center gap-2 text-sm text-destructive">
      <AlertCircle className="size-4 shrink-0" />
      <span>Couldn't detect agent harnesses.</span>
      <Button onClick={retryBootWarm} size="sm" variant="outline">
        Try again
      </Button>
    </div>
  );
}
