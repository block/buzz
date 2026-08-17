import type {
  ProjectIssue,
  ProjectPullRequest,
  Repository,
} from "@/features/projects/hooks";
import {
  type ProjectSelectionItem,
  selectionItemFromCommit,
  selectionItemFromReview,
  selectionItemFromTask,
} from "@/features/projects/lib/projectSelection";
import {
  commitShareLink,
  issueShareLink,
  pullRequestShareLink,
} from "@/features/projects/lib/projectShareLinks";
import type { ProjectRepoCommit } from "@/shared/api/types";

export function projectDetailSelectionItem({
  commit,
  issue,
  projectId,
  pullRequest,
  repository,
}: {
  commit?: ProjectRepoCommit | null;
  issue?: ProjectIssue | null;
  projectId: string;
  pullRequest?: ProjectPullRequest | null;
  repository: Repository;
}): ProjectSelectionItem | null {
  if (issue) {
    return selectionItemFromTask({
      author: issue.author,
      channelId: issue.channelId ?? repository.channelId,
      id: issue.id,
      shareLink: issueShareLink(issue),
      title: issue.title,
    });
  }
  if (pullRequest) {
    return selectionItemFromReview({
      author: pullRequest.author,
      channelId: pullRequest.channelId ?? repository.channelId,
      id: pullRequest.id,
      shareLink: pullRequestShareLink(pullRequest),
      title: pullRequest.title,
    });
  }
  if (commit) {
    return selectionItemFromCommit({
      channelId: repository.channelId,
      commitHash: commit.hash,
      projectId,
      shareLink: commitShareLink(repository, commit.hash),
      title: commit.subject,
    });
  }
  return null;
}
