import type {
  Project,
  ProjectIssue,
  ProjectPullRequest,
} from "@/features/projects/hooks";
import type { ProjectsWorkItemsResult } from "@/features/projects/projectWorkItems";
import type { FeedItem } from "@/shared/api/types";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";

const PROJECT_ROOT_KINDS = new Set([KIND_GIT_PULL_REQUEST, KIND_GIT_ISSUE]);
const PROJECT_ACTIVITY_KINDS = new Set([
  KIND_TEXT_NOTE,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_STATUS_OPEN,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
]);
const REPO_ADDRESS_PATTERN = /^30617:[0-9a-f]{64}:.+$/i;

export type ProjectInboxWorkItem =
  | {
      type: "pull-request";
      project: Project;
      pullRequest: ProjectPullRequest;
    }
  | {
      type: "issue";
      project: Project;
      issue: ProjectIssue;
    };

function tagValue(item: Pick<FeedItem, "tags">, name: string) {
  return item.tags.find(
    (tag) => tag[0] === name && typeof tag[1] === "string" && tag[1].length > 0,
  )?.[1];
}

/** Returns the canonical Buzz Git repository and root event for an Inbox row. */
export function getProjectInboxReference(
  item: Pick<FeedItem, "id" | "kind" | "tags">,
): { repoAddress: string; rootId: string } | null {
  const repoAddress = tagValue(item, "a");
  if (!repoAddress || !REPO_ADDRESS_PATTERN.test(repoAddress)) {
    return null;
  }

  if (PROJECT_ROOT_KINDS.has(item.kind)) {
    return { repoAddress, rootId: item.id };
  }

  if (!PROJECT_ACTIVITY_KINDS.has(item.kind)) {
    return null;
  }

  const rootId = tagValue(item, "e") ?? tagValue(item, "E");
  return rootId ? { repoAddress, rootId } : null;
}

/** Whether a feed event belongs to a Buzz Git pull request or issue thread. */
export function isProjectInboxItem(item: FeedItem) {
  return getProjectInboxReference(item) !== null;
}

/** Resolves an Inbox event to the current canonical Buzz Git work item. */
export function resolveProjectInboxWorkItem(
  item: FeedItem,
  workItems: ProjectsWorkItemsResult<Project> | undefined,
): ProjectInboxWorkItem | null {
  const reference = getProjectInboxReference(item);
  if (!reference || !workItems) {
    return null;
  }

  const pullRequestEntry = workItems.pullRequests.items.find(
    ({ project, pullRequest }) =>
      project.repoAddress === reference.repoAddress &&
      pullRequest.id === reference.rootId,
  );
  if (pullRequestEntry) {
    return { type: "pull-request", ...pullRequestEntry };
  }

  const issueEntry = workItems.issues.items.find(
    ({ issue, project }) =>
      project.repoAddress === reference.repoAddress &&
      issue.id === reference.rootId,
  );
  return issueEntry ? { type: "issue", ...issueEntry } : null;
}
