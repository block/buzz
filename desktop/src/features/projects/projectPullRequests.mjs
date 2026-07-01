import { getAllTags, getTag } from "./projectIssues.mjs";

function latestUpdateForPullRequest(pullRequestId, updateEvents) {
  return updateEvents
    .filter((event) => getTag(event, "E") === pullRequestId)
    .sort((left, right) => right.created_at - left.created_at)[0];
}

function latestStatusForPullRequest(pullRequestId, statusEvents) {
  return statusEvents
    .filter((event) =>
      event.tags.some(
        (tag) => (tag[0] === "e" || tag[0] === "E") && tag[1] === pullRequestId,
      ),
    )
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

function statusFromEvent(pullRequest, statusEvent) {
  if (statusEvent?.kind === 1630) return "Open";
  if (statusEvent?.kind === 1631) return "Merged";
  if (statusEvent?.kind === 1632) return "Closed";
  if (statusEvent?.kind === 1633) return "Draft";
  const labels = getAllTags(pullRequest, "t").map((label) =>
    label.toLowerCase(),
  );
  return labels.includes("draft") ? "Draft" : "Open";
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
  statusEvents = [],
) {
  const latestUpdate = latestUpdateForPullRequest(pullRequest.id, updateEvents);
  const latestStatus = latestStatusForPullRequest(pullRequest.id, statusEvents);
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
    status: statusFromEvent(pullRequest, latestStatus),
    statusEventId: latestStatus?.id ?? null,
    branchName: getTag(pullRequest, "branch-name") ?? null,
    commit: latestCommit,
    cloneUrls: getCloneUrls(latestUpdate ?? pullRequest),
    updateCount: updates.length,
    updatedAt:
      [
        ...updates,
        ...comments,
        ...(latestStatus
          ? [
              {
                createdAt: latestStatus.created_at,
              },
            ]
          : []),
      ].sort((left, right) => right.createdAt - left.createdAt)[0]?.createdAt ??
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
  statusEvents = [],
) {
  return [...pullRequestEvents]
    .map((pullRequest) =>
      eventToProjectPullRequest(
        pullRequest,
        updateEvents,
        commentEvents,
        statusEvents,
      ),
    )
    .sort((left, right) => right.updatedAt - left.updatedAt);
}
