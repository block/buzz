import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import type { Repository as Project } from "@/features/projects/hooks";

/** Shared by every BW write panel (`BwIssueActions.tsx`,
 * `BwAssignmentSection.tsx`): refresh both the per-project issue list and
 * the cross-project work-items view after a write succeeds. */
export function useInvalidateProjectIssues(project: Project) {
  const queryClient = useQueryClient();
  return React.useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: ["project", project.id, "issues"],
      }),
      queryClient.invalidateQueries({ queryKey: ["projects", "work-items"] }),
    ]);
  }, [project.id, queryClient]);
}

export function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}
