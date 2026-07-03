import { allowedActorsForRoot, getAllTags, getTag } from "./projectIssues.mjs";

// Updates and status changes rewrite the PR's tip commit, clone URLs, and
// lifecycle state, so they are only honored when signed by the PR author or
// the repo owner — an arbitrary relay user must not be able to re-point an
// open PR at their own commit/clone URL or flip its status.
function trustedUpdatesForPullRequest(pullRequest, updateEvents) {
  const allowedActors = allowedActorsForRoot(pullRequest);
  return updateEvents.filter(
    (event) =>
      allowedActors.has(event.pubkey.toLowerCase()) &&
      getTag(event, "E") === pullRequest.id,
  );
}

function latestUpdateForPullRequest(pullRequest, updateEvents) {
  return trustedUpdatesForPullRequest(pullRequest, updateEvents).sort(
    (left, right) => right.created_at - left.created_at,
  )[0];
}

function latestStatusForPullRequest(pullRequest, statusEvents) {
  const allowedActors = allowedActorsForRoot(pullRequest);
  return statusEvents
    .filter(
      (event) =>
        allowedActors.has(event.pubkey.toLowerCase()) &&
        event.tags.some(
          (tag) =>
            (tag[0] === "e" || tag[0] === "E") && tag[1] === pullRequest.id,
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
  const latestUpdate = latestUpdateForPullRequest(pullRequest, updateEvents);
  const latestStatus = latestStatusForPullRequest(pullRequest, statusEvents);
  const updates = eventsForPullRequest(
    pullRequest.id,
    trustedUpdatesForPullRequest(pullRequest, updateEvents),
  ).map(eventToPullRequestUpdate);
  const comments = eventsForPullRequest(pullRequest.id, commentEvents).map(
    eventToPullRequestComment,
  );
  const title =
    getTag(pullRequest, "subject") ||
    pullRequest.content.split("\n")[0] ||
    "Untitled pull request";
  const latestCommit = getTag(latestUpdate ?? pullRequest, "c") ?? null;
  const initialCommit = getTag(pullRequest, "c") ?? null;

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
    initialCommit,
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
