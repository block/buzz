import { useMutation } from "@tanstack/react-query";

import {
  mergeProjectPullRequest,
  publishProjectPullRequestMergedStatus,
} from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Project, ProjectPullRequest } from "./hooks";
import { nextProjectPullRequestStatusCreatedAt } from "./projectPullRequests.mjs";
import { useProjectPullRequestWriteInvalidation } from "./pullRequestReviews";

type CreateProjectPullRequestInput = {
  title: string;
  body: string;
  branch: string;
  targetBranch: string;
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
    ["target-branch", input.targetBranch],
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

/** Whether the active identity may publish a trusted update for this PR. */
export function canPublishProjectPullRequestUpdate(
  viewerPubkey: string,
  project: Project,
  pullRequest: ProjectPullRequest,
) {
  const viewer = normalizePubkey(viewerPubkey);
  return (
    viewer === normalizePubkey(project.owner) ||
    viewer === normalizePubkey(pullRequest.author)
  );
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
  if (input.branch === input.targetBranch) {
    throw new Error("The base and compare branches must be different.");
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
  const identity = await getIdentity();
  if (
    !canPublishProjectPullRequestUpdate(identity.pubkey, project, pullRequest)
  ) {
    throw new Error(
      "Only the pull request author or repository owner can publish its update.",
    );
  }
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
  statusEvent: string,
) {
  await publishProjectPullRequestMergedStatus({
    targetOwner: project.owner,
    statusEvent,
  });
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
        repoAddress: project.repoAddress,
        pullRequestId: pullRequest.id,
        pullRequestAuthor: pullRequest.author,
        statusCreatedAt: nextProjectPullRequestStatusCreatedAt(
          pullRequest,
          Math.floor(Date.now() / 1_000),
        ),
        targetBranch: pullRequest.targetBranch ?? project.defaultBranch,
        sourceBranch: pullRequest.branchName,
        expectedCommit: pullRequest.commit,
      });
      return result;
    },
    onSuccess: invalidate,
  });
}

export function usePublishProjectPullRequestMergedMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);
  return useMutation({
    mutationFn: ({ statusEvent }: { statusEvent: string }) => {
      if (!project) throw new Error("No project selected.");
      return publishProjectPullRequestMerged(project, statusEvent);
    },
    onSuccess: invalidate,
  });
}
