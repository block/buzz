import { getAllTags, getTag } from "./projectIssues.mjs";

function latestUpdateForPullRequest(pullRequestId, updateEvents) {
  return updateEvents
    .filter((event) => getTag(event, "E") === pullRequestId)
    .sort((left, right) => right.created_at - left.created_at)[0];
}

function eventsForPullRequest(pullRequestId, events) {
  return events
    .filter((event) =>
      event.tags.some(
        (tag) => (tag[0] === "e" || tag[0] === "E") && tag[1] === pullRequestId,
      ),
    )
    .sort((left, right) => left.created_at - right.created_at);
}

function getCloneUrls(event) {
  return event.tags
    .filter((tag) => tag[0] === "clone")
    .flatMap((tag) => tag.slice(1))
    .filter(Boolean);
}

function eventToPullRequestUpdate(event) {
  return {
    id: event.id,
    content: event.content,
    author: event.pubkey,
    createdAt: event.created_at,
    commit: getTag(event, "c") ?? null,
    cloneUrls: getCloneUrls(event),
  };
}

function eventToPullRequestComment(event) {
  return {
    id: event.id,
    content: event.content,
    author: event.pubkey,
    createdAt: event.created_at,
  };
}

export function eventToProjectPullRequest(
  pullRequest,
  updateEvents = [],
  commentEvents = [],
) {
  const latestUpdate = latestUpdateForPullRequest(pullRequest.id, updateEvents);
  const updates = eventsForPullRequest(pullRequest.id, updateEvents).map(
    eventToPullRequestUpdate,
  );
  const comments = eventsForPullRequest(pullRequest.id, commentEvents).map(
    eventToPullRequestComment,
  );
  const title =
    getTag(pullRequest, "subject") ||
    pullRequest.content.split("\n")[0] ||
    "Untitled pull request";
  const latestCommit = getTag(latestUpdate ?? pullRequest, "c") ?? null;

  return {
    id: pullRequest.id,
    title,
    content: pullRequest.content,
    author: pullRequest.pubkey,
    createdAt: pullRequest.created_at,
    repoAddress: getTag(pullRequest, "a") ?? null,
    labels: getAllTags(pullRequest, "t"),
    recipients: getAllTags(pullRequest, "p"),
    branchName: getTag(pullRequest, "branch-name") ?? null,
    commit: latestCommit,
    cloneUrls: getCloneUrls(latestUpdate ?? pullRequest),
    updateCount: updates.length,
    updatedAt:
      [...updates, ...comments].sort(
        (left, right) => right.createdAt - left.createdAt,
      )[0]?.createdAt ??
      latestUpdate?.created_at ??
      pullRequest.created_at,
    updates,
    comments,
  };
}

export function projectPullRequestEventsToPullRequests(
  pullRequestEvents,
  updateEvents = [],
  commentEvents = [],
) {
  return [...pullRequestEvents]
    .map((pullRequest) =>
      eventToProjectPullRequest(pullRequest, updateEvents, commentEvents),
    )
    .sort((left, right) => right.updatedAt - left.updatedAt);
}
