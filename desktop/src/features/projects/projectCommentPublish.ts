import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";

import type { ProjectIssue } from "./projectIssues.mjs";
import { projectCommentKindAndChannelTags } from "./projectCommentPublish.mjs";
import type {
  ProjectPullRequest,
  ProjectPullRequestCommentAnchor,
} from "./projectPullRequests.mjs";
import {
  normalizeProjectPullRequestCommentAnchor,
  PR_INLINE_COMMENT_LABEL,
} from "./projectPullRequests.mjs";

export { projectCommentKindAndChannelTags } from "./projectCommentPublish.mjs";

type ProjectCommentTarget = {
  owner: string;
  projectChannelId: string | null;
  repoAddress: string;
};

export async function createProjectPullRequestComment({
  anchor,
  content,
  mediaTags,
  mentionPubkeys = [],
  project,
  pullRequest,
}: {
  anchor?: ProjectPullRequestCommentAnchor;
  content: string;
  mediaTags?: string[][];
  mentionPubkeys?: string[];
  project: ProjectCommentTarget;
  pullRequest: ProjectPullRequest;
}): Promise<void> {
  const body = content.trim();
  if (!body) {
    throw new Error("Comment cannot be empty.");
  }
  const normalizedAnchor = anchor
    ? normalizeProjectPullRequestCommentAnchor(anchor)
    : null;
  if (anchor && !normalizedAnchor) {
    throw new Error("Comment location is invalid.");
  }
  if (normalizedAnchor && !pullRequest.commit) {
    throw new Error("Pull request commit is required for inline comments.");
  }

  const { kind, channelTags } = projectCommentKindAndChannelTags(
    project,
    mentionPubkeys,
  );
  const recipients = new Set([
    project.owner.toLowerCase(),
    pullRequest.author.toLowerCase(),
    ...pullRequest.recipients.map((recipient) => recipient.toLowerCase()),
    ...mentionPubkeys.map((pubkey) => pubkey.toLowerCase()),
  ]);
  const tags = [
    ...channelTags,
    ["e", pullRequest.id, "", "root"],
    ["a", project.repoAddress],
    ...[...recipients].map((recipient) => ["p", recipient]),
    ...(normalizedAnchor
      ? [
          ["t", PR_INLINE_COMMENT_LABEL],
          ["c", pullRequest.commit as string],
          ["file", normalizedAnchor.path],
          ["side", normalizedAnchor.side],
          ["line", String(normalizedAnchor.line)],
        ]
      : []),
    ...(mediaTags ?? []),
  ];

  const event = await signRelayEvent({
    kind,
    content: body,
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out posting pull request comment.",
    "Failed to post pull request comment.",
  );
}

export async function createProjectIssueComment({
  content,
  mediaTags,
  mentionPubkeys = [],
  issue,
  project,
}: {
  content: string;
  mediaTags?: string[][];
  mentionPubkeys?: string[];
  issue: ProjectIssue;
  project: ProjectCommentTarget;
}): Promise<void> {
  const body = content.trim();
  if (!body) {
    throw new Error("Comment cannot be empty.");
  }

  const { kind, channelTags } = projectCommentKindAndChannelTags(
    project,
    mentionPubkeys,
  );
  const recipients = new Set([
    project.owner.toLowerCase(),
    issue.author.toLowerCase(),
    ...issue.recipients.map((recipient) => recipient.toLowerCase()),
    ...mentionPubkeys.map((pubkey) => pubkey.toLowerCase()),
  ]);
  const tags = [
    ...channelTags,
    ["e", issue.id, "", "root"],
    ["a", project.repoAddress],
    ...[...recipients].map((recipient) => ["p", recipient]),
    ...(mediaTags ?? []),
  ];

  const event = await signRelayEvent({
    kind,
    content: body,
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out posting issue comment.",
    "Failed to post issue comment.",
  );
}
