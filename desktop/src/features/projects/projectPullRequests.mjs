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

/** Keep consecutive lifecycle writes ordered even when they happen within the
 * same whole-second Nostr timestamp. */
export function nextProjectPullRequestStatusCreatedAt(pullRequest, now) {
  return Math.max(now, (pullRequest.statusCreatedAt ?? 0) + 1);
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

// Review requests and approvals are kind:1 comments labeled with a `t` tag —
// NIP-34 has no dedicated review kinds, and labeled text notes stay readable
// for any client (including `buzz` CLI users) that treats them as comments.
export const PR_REVIEW_REQUEST_LABEL = "review-request";
export const PR_APPROVAL_LABEL = "approval";

function eventToPullRequestComment(event) {
  const labels = getAllTags(event, "t").map((label) => label.toLowerCase());
  const isReviewRequest = labels.includes(PR_REVIEW_REQUEST_LABEL);
  return {
    id: event.id,
    content: event.content,
    author: event.pubkey,
    createdAt: event.created_at,
    isApproval: labels.includes(PR_APPROVAL_LABEL),
    isReviewRequest,
    // For review requests the `p` tags are the requested reviewers.
    reviewerPubkeys: isReviewRequest
      ? getAllTags(event, "p").map((pubkey) => pubkey.toLowerCase())
      : [],
  };
}

/**
 * Requested reviewers: `p` tags on the PR root plus `p` tags of trusted
 * review-request comments (signed by the PR author or repo owner). The PR
 * author is never their own reviewer.
 */
function reviewersForPullRequest(pullRequest, comments) {
  const allowedActors = allowedActorsForRoot(pullRequest);
  const reviewers = new Set(
    getAllTags(pullRequest, "p").map((pubkey) => pubkey.toLowerCase()),
  );
  for (const comment of comments) {
    if (
      comment.isReviewRequest &&
      allowedActors.has(comment.author.toLowerCase())
    ) {
      for (const pubkey of comment.reviewerPubkeys) {
        reviewers.add(pubkey);
      }
    }
  }
  reviewers.delete(pullRequest.pubkey.toLowerCase());
  return [...reviewers];
}

/** Latest approval comment per author, oldest first. */
function approvalsForPullRequest(comments) {
  const byAuthor = new Map();
  for (const comment of comments) {
    if (!comment.isApproval) continue;
    const key = comment.author.toLowerCase();
    const existing = byAuthor.get(key);
    if (!existing || comment.createdAt > existing.createdAt) {
      byAuthor.set(key, comment);
    }
  }
  return [...byAuthor.values()]
    .map(({ id, author, createdAt }) => ({ id, author, createdAt }))
    .sort((left, right) => left.createdAt - right.createdAt);
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
  const reviewers = reviewersForPullRequest(pullRequest, comments);
  const approvals = approvalsForPullRequest(comments);
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
    reviewers,
    approvals,
    status: statusFromEvent(pullRequest, latestStatus),
    statusEventId: latestStatus?.id ?? null,
    statusCreatedAt: latestStatus?.created_at ?? null,
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
