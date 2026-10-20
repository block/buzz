import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { fetchProjectTask } from "@/features/projects/fetchProjectTask";
import { useProjectsQuery } from "@/features/projects/hooks";
import type { ProjectIssue } from "@/features/projects/projectIssues.mjs";
import type { CatchUpRecap } from "@/shared/api/tauriThreadCatchUp";
import {
  type CatchUpAssignedTask,
  eligibleCatchUpRepoAddresses,
  extractCatchUpTaskReferences,
  selectCatchUpAssignedTasks,
} from "./threadCatchUp";

export type CatchUpAssignments =
  | { status: "none" }
  | { status: "loading" }
  | { status: "error" }
  | {
      status: "ready";
      tasks: CatchUpAssignedTask[];
      /** Linked tasks whose state could not be verified. */
      unverifiedCount: number;
    };

/**
 * "Assigned to you" for one recap: tasks explicitly linked from the recap's
 * own source messages, in repositories of projects the reader can see, whose
 * reduced assignment state currently names the reader. Nothing is inferred
 * from prose, mentions, or the model's output, and nothing is changed.
 */
export function useThreadCatchUpAssignments({
  recap,
  bodies,
  currentPubkey,
}: {
  recap: CatchUpRecap | null;
  bodies: ReadonlyMap<string, string>;
  currentPubkey: string;
}): CatchUpAssignments {
  const references = React.useMemo(() => {
    if (!recap) return [];
    const messages = recap.sourceIds.flatMap((id) => {
      const body = bodies.get(id);
      return body === undefined ? [] : [{ id, body }];
    });
    return extractCatchUpTaskReferences(messages, new Set(recap.sourceIds));
  }, [bodies, recap]);

  const projectsQuery = useProjectsQuery(references.length > 0);
  const eligibleRepos = React.useMemo(
    () => eligibleCatchUpRepoAddresses(projectsQuery.data ?? []),
    [projectsQuery.data],
  );
  const candidates = React.useMemo(
    () =>
      references.filter((reference) =>
        eligibleRepos.has(reference.repoAddress),
      ),
    [eligibleRepos, references],
  );

  const tasksQuery = useQuery({
    queryKey: [
      "thread-catch-up",
      "tasks",
      candidates.map((reference) => reference.issueId),
    ],
    enabled: candidates.length > 0,
    staleTime: 30_000,
    queryFn: async ({ signal }) => {
      const settled = await Promise.allSettled(
        candidates.map((reference) =>
          fetchProjectTask(
            reference.issueId,
            reference.repoAddress,
            undefined,
            signal,
          ),
        ),
      );
      const tasks = new Map<string, ProjectIssue | null>();
      let unverifiedCount = 0;
      settled.forEach((result, index) => {
        if (result.status === "fulfilled") {
          tasks.set(candidates[index].issueId, result.value);
        } else {
          unverifiedCount += 1;
        }
      });
      return { tasks, unverifiedCount };
    },
  });

  if (!recap || references.length === 0) return { status: "none" };
  if (projectsQuery.isPending) return { status: "loading" };
  if (projectsQuery.isError) return { status: "error" };
  if (candidates.length === 0) {
    return { status: "ready", tasks: [], unverifiedCount: 0 };
  }
  if (tasksQuery.isPending) return { status: "loading" };
  if (tasksQuery.isError) return { status: "error" };
  return {
    status: "ready",
    tasks: selectCatchUpAssignedTasks({
      references: candidates,
      tasks: tasksQuery.data.tasks,
      eligibleRepoAddresses: eligibleRepos,
      currentPubkey,
    }),
    unverifiedCount: tasksQuery.data.unverifiedCount,
  };
}
