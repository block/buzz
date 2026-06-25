import { Check, X } from "lucide-react";

import { Spinner } from "@/shared/ui/spinner";

export type ImportItemStatus = "pending" | "importing" | "done" | "error";

/**
 * Tiny status indicator used in sequential-import dialogs (persona batch
 * import, team import). Renders a spinner while importing, a check on
 * success, an X on failure, and nothing while pending.
 */
export function ImportStatusIcon({
  status,
}: {
  status: ImportItemStatus | undefined;
}) {
  switch (status) {
    case "importing":
      return (
        <Spinner className="h-4 w-4 shrink-0 border-2 text-muted-foreground" />
      );
    case "done":
      return <Check className="h-4 w-4 shrink-0 text-green-500" />;
    case "error":
      return <X className="h-4 w-4 shrink-0 text-destructive" />;
    default:
      return null;
  }
}
