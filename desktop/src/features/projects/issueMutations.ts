import { useMutation, useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { KIND_GIT_ISSUE } from "@/shared/constants/kinds";
import type { Repository as Project } from "./hooks";
import { buildGitIssueTags } from "./projectIssues.mjs";
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
