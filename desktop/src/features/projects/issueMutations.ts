import { useMutation, useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { KIND_GIT_ISSUE } from "@/shared/constants/kinds";
import type { Repository as Project } from "./hooks";
import { useProjectIssueWriteInvalidation } from "./issueAssignments";
import {
  buildGitIssueTags,
  buildProjectIssueStatusEventTemplate,
  type ProjectIssue,
  type ProjectIssueLifecycleStatus,
} from "./projectIssues.mjs";
import type { ProjectTaskCategory } from "./projectTaskCategories";

type CreateProjectIssueInput = {
  title: string;
  body: string;
  category?: ProjectTaskCategory;
};

export async function publishProjectIssue(
  project: Project,
  input: CreateProjectIssueInput,
) {
  const event = await signRelayEvent({
    kind: KIND_GIT_ISSUE,
    content: input.body.trim(),
    tags: buildGitIssueTags({
      repoAddress: project.repoAddress,
      repoOwner: project.owner,
      title: input.title,
      labels: [input.category ?? "issue"],
    }),
  });
  await relayClient.publishEvent(
    event,
    "Timed out creating task.",
    "Failed to create task.",
  );
  return event.id;
}

export function useCreateProjectIssueMutation(
  project: Project | null | undefined,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateProjectIssueInput) => {
      if (!project) throw new Error("No project selected.");
      return publishProjectIssue(project, input);
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["project", project?.id ?? "none", "issues"],
        }),
        queryClient.invalidateQueries({
          queryKey: ["projects", "work-items"],
        }),
        queryClient.invalidateQueries({
          queryKey: ["projects", "activity-summaries"],
        }),
      ]);
    },
  });
}

// Same trust rule as PR status changes (allowedActorsForRoot): only the issue
// author or repo owner are honored by the status reduction, so the event is
// published as the signed-in identity and simply won't take effect for others.
export async function updateProjectIssueStatus(
  project: Project,
  issue: ProjectIssue,
  status: ProjectIssueLifecycleStatus,
): Promise<void> {
  const event = await signRelayEvent(
    buildProjectIssueStatusEventTemplate({
      issue,
      now: Math.floor(Date.now() / 1_000),
      repoAddress: project.repoAddress,
      repoOwner: project.owner,
      status,
    }),
  );
  await relayClient.publishEvent(
    event,
    "Timed out updating task status.",
    "Failed to update task status.",
  );
}

export function useUpdateProjectIssueStatusMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectIssueWriteInvalidation(project);
  return useMutation({
    mutationFn: ({
      issue,
      status,
    }: {
      issue: ProjectIssue;
      status: ProjectIssueLifecycleStatus;
    }) => {
      if (!project) throw new Error("No project selected.");
      return updateProjectIssueStatus(project, issue, status);
    },
    onSuccess: invalidate,
  });
}
