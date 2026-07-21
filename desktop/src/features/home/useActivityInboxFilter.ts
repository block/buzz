import * as React from "react";

import type { InboxFilter } from "@/features/home/lib/inbox";

export function useActivityInboxFilter(activityEnabled: boolean) {
  const [filter, setFilter] = React.useState<InboxFilter>("all");

  React.useEffect(() => {
    if (activityEnabled && filter === "activity") setFilter("all");
  }, [activityEnabled, filter]);

  return [filter, setFilter] as const;
}
