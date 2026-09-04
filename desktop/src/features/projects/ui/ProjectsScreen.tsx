import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { ProjectsView } from "@/features/projects/ui/ProjectsView";

export function ProjectsScreen() {
  const queryClient = useQueryClient();
  React.useEffect(() => {
    return () => {
      // Leaving the surface: stop the work-items fan. Its assignment-operation
      // pagination is abort-aware and unbounded, so cancellation genuinely
      // stops relay work that would otherwise compete with the next surface's
      // channel fetches. Cached data stays; the next visit refetches per
      // staleTime.
      //
      // Deliberately NOT cancelled:
      // - projects enumeration: the always-mounted sidebar projects section
      //   observes this key, so it must keep filling its 30-minute cache.
      // - repo snapshots / local repositories: native clone/scan work cannot
      //   be aborted mid-flight; cancellation would only discard the finished
      //   result and force the same clones again on the next visit.
      // - activity summaries: a single bounded request — it completes either
      //   way, so cancelling just wastes the response.
      void queryClient.cancelQueries({ queryKey: ["projects", "work-items"] });
    };
  }, [queryClient]);

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <ProjectsView />
    </div>
  );
}
