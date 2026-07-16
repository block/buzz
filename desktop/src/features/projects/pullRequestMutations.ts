import { useMutation } from "@tanstack/react-query";

import { mergeProjectPullRequest } from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import {
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_MERGED,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Project, ProjectPullRequest } from "./hooks";
import { nextProjectPullRequestStatusCreatedAt } from "./projectPullRequests.mjs";
import { useProjectPullRequestWriteInvalidation } from "./pullRequestReviews";

type CreateProjectPullRequestInput = {
  title: string;
  body: string;
  branch: string;
  commit: string;
  mergeBase: string | null;
  reviewers: string[];
};

function uniquePubkeys(pubkeys: readonly string[]) {
  return [...new Set(pubkeys.map(normalizePubkey))];
}

export function projectPullRequestTags(
  project: Project,
  input: CreateProjectPullRequestInput,
): string[][] {
  const tags = [
    ["a", project.repoAddress],
    ...uniquePubkeys([project.owner, ...input.reviewers]).map((pubkey) => [
      "p",
      pubkey,
    ]),
    ["subject", input.title],
    ["c", input.commit],
    ["clone", ...project.cloneUrls],
    ["branch-name", input.branch],
  ];
  if (input.mergeBase) tags.push(["merge-base", input.mergeBase]);
  return tags;
}

export function projectPullRequestUpdateTags(
  project: Project,
  pullRequest: ProjectPullRequest,
  commit: string,
  mergeBase: string | null,
): string[][] {
  const tags = [
    ["a", project.repoAddress],
    ...uniquePubkeys([project.owner, pullRequest.author]).map((pubkey) => [
      "p",
      pubkey,
    ]),
    ["E", pullRequest.id],
    ["P", normalizePubkey(pullRequest.author)],
    ["c", commit],
    [
      "clone",
      ...(pullRequest.cloneUrls.length > 0
        ? pullRequest.cloneUrls
        : project.cloneUrls),
    ],
  ];
  if (mergeBase) tags.push(["merge-base", mergeBase]);
  return tags;
}

export function projectPullRequestMergedTags(
  project: Project,
  pullRequest: ProjectPullRequest,
  mergeCommit: string,
): string[][] {
  return [
    ["e", pullRequest.id, "", "root"],
    ["a", project.repoAddress],
    ...uniquePubkeys([project.owner, pullRequest.author]).map((pubkey) => [
      "p",
      pubkey,
    ]),
    ["merge-commit", mergeCommit],
    ["r", mergeCommit],
  ];
}

async function publishProjectPullRequest(
  project: Project,
  input: CreateProjectPullRequestInput,
) {
  const title = input.title.trim();
  if (!title) throw new Error("Pull request title cannot be empty.");
  if (title.length > 256) {
    throw new Error("Pull request title must be 256 characters or fewer.");
  }
  if (project.cloneUrls.length === 0) {
    throw new Error("This project has no clone URL.");
  }
  if (input.branch === project.defaultBranch) {
    throw new Error("Choose a branch other than the default branch.");
  }

  const event = await signRelayEvent({
    kind: KIND_GIT_PULL_REQUEST,
    content: input.body.trim(),
    tags: projectPullRequestTags(project, { ...input, title }),
  });
  await relayClient.publishEvent(
    event,
    "Timed out creating pull request.",
    "Failed to create pull request.",
  );
  return event.id;
}

export async function publishProjectPullRequestUpdate({
  commit,
  mergeBase,
  project,
  pullRequest,
}: {
  commit: string;
  mergeBase: string | null;
  project: Project;
  pullRequest: ProjectPullRequest;
}): Promise<boolean> {
  if (pullRequest.commit?.toLowerCase() === commit.toLowerCase()) return false;
  const event = await signRelayEvent({
    kind: KIND_GIT_PR_UPDATE,
    content: "",
    createdAt: Math.max(
      Math.floor(Date.now() / 1_000),
      ...pullRequest.updates.map((update) => update.createdAt + 1),
    ),
    tags: projectPullRequestUpdateTags(project, pullRequest, commit, mergeBase),
  });
  await relayClient.publishEvent(
    event,
    "Timed out updating pull request.",
    "The branch was pushed, but the pull request update could not be published.",
  );
  return true;
}

export async function publishProjectPullRequestMerged(
  project: Project,
  pullRequest: ProjectPullRequest,
  mergeCommit: string,
) {
  const event = await signRelayEvent({
    kind: KIND_GIT_STATUS_MERGED,
    content: "",
    createdAt: nextProjectPullRequestStatusCreatedAt(
      pullRequest,
      Math.floor(Date.now() / 1_000),
    ),
    tags: projectPullRequestMergedTags(project, pullRequest, mergeCommit),
  });
  await relayClient.publishEvent(
    event,
    "Repository merged, but publishing its pull request status timed out.",
    "Repository merged, but its pull request status could not be published.",
  );
}

export function useCreateProjectPullRequestMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);
  return useMutation({
    mutationFn: (input: CreateProjectPullRequestInput) => {
      if (!project) throw new Error("No project selected.");
      return publishProjectPullRequest(project, input);
    },
    onSuccess: invalidate,
  });
}

export function useUpdateProjectPullRequestMutation(
  project: Project | null | undefined,
  pullRequest: ProjectPullRequest | null,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);
  return useMutation({
    mutationFn: async ({
      commit,
      mergeBase,
    }: {
      commit: string;
      mergeBase: string | null;
    }) => {
      if (!project) throw new Error("No project selected.");
      if (!pullRequest)
        throw new Error("No open pull request for this branch.");
      return publishProjectPullRequestUpdate({
        commit,
        mergeBase,
        project,
        pullRequest,
      });
    },
    onSuccess: invalidate,
  });
}

export function useMergeProjectPullRequestMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);
  return useMutation({
    mutationFn: async ({
      pullRequest,
    }: {
      pullRequest: ProjectPullRequest;
    }) => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      if (!pullRequest.branchName || !pullRequest.commit) {
        throw new Error("Pull request branch information is incomplete.");
      }
      const result = await mergeProjectPullRequest({
        targetCloneUrl: project.cloneUrls[0],
        sourceCloneUrl: pullRequest.cloneUrls[0] ?? project.cloneUrls[0],
        targetOwner: project.owner,
        targetBranch: project.defaultBranch,
        sourceBranch: pullRequest.branchName,
        expectedCommit: pullRequest.commit,
      });
      let statusPublicationError: string | null = null;
      try {
        await publishProjectPullRequestMerged(
          project,
          pullRequest,
          result.mergeCommit,
        );
      } catch (error) {
        statusPublicationError =
          error instanceof Error
            ? error.message
            : "Pull request status could not be published.";
      }
      return { ...result, statusPublicationError };
    },
    onSuccess: invalidate,
  });
}

export function usePublishProjectPullRequestMergedMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);
  return useMutation({
    mutationFn: ({
      mergeCommit,
      pullRequest,
    }: {
      mergeCommit: string;
      pullRequest: ProjectPullRequest;
    }) => {
      if (!project) throw new Error("No project selected.");
      return publishProjectPullRequestMerged(project, pullRequest, mergeCommit);
    },
    onSuccess: invalidate,
  });
}
