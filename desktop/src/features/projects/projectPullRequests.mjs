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

/** Keep consecutive decisions ordered across whole-second Nostr timestamps. */
export function nextProjectPullRequestReviewCreatedAt(pullRequest, now) {
  const latestDecisionCreatedAt = [
    ...pullRequest.approvals,
    ...pullRequest.changeRequests,
  ].reduce((latest, decision) => Math.max(latest, decision.createdAt), 0);
  return Math.max(now, latestDecisionCreatedAt + 1);
}

/** Trusted presentation kind for a compact review timeline row. */
export function projectPullRequestCommentTimelineKind(comment) {
  if (comment.isTrustedReviewRequest) return "review-request";
  if (
    !comment.isTrustedReviewDecision ||
    !comment.reviewDecisionStatus ||
    !comment.reviewDecision
  ) {
    return null;
  }
  return comment.reviewDecision;
}

/** Effective review summary shown above the PR review actions. */
export function projectPullRequestReviewSummary(pullRequest) {
  const approvalCount = pullRequest.approvals.length;
  const changeRequestCount = pullRequest.changeRequests.length;
  const isDraft = pullRequest.status === "Draft";
  const state = isDraft
    ? "This pull request is still a work in progress."
    : changeRequestCount > 0
      ? `${changeRequestCount} reviewer${changeRequestCount === 1 ? "" : "s"} requested changes.`
      : pullRequest.reviewers.length > 0
        ? "Review requested — no approvals yet."
        : "No reviews yet.";

  return {
    approvalCount,
    changeRequestCount,
    detail: isDraft
      ? "Draft pull requests cannot be merged."
      : approvalCount === 0 && changeRequestCount === 0
        ? "Approvals from reviewers will show up here."
        : null,
    showState: approvalCount === 0 || changeRequestCount > 0,
    state,
  };
}

/** Effective trusted, current-commit review decision represented by a comment. */
export function projectPullRequestEffectiveReviewDecision(
  pullRequest,
  comment,
) {
  if (pullRequest.approvals.some((decision) => decision.id === comment.id)) {
    return "approved";
  }
  if (
    pullRequest.changeRequests.some((decision) => decision.id === comment.id)
  ) {
    return "changes-requested";
  }
  return null;
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
export const PR_CHANGES_REQUESTED_LABEL = "changes-requested";
export const PR_INLINE_COMMENT_LABEL = "inline-comment";

/** Validate an inline diff anchor without normalizing attacker-controlled paths. */
export function normalizeProjectPullRequestCommentAnchor(anchor) {
  if (!anchor || typeof anchor.path !== "string") return null;
  const path = anchor.path;
  if (
    path.length === 0 ||
    path.length > 4_096 ||
    path.startsWith("/") ||
    path.includes("\\") ||
    path.includes("\0") ||
    path
      .split("/")
      .some((segment) => !segment || segment === "." || segment === "..")
  ) {
    return null;
  }
  if (anchor.side !== "old" && anchor.side !== "new") return null;
  if (!Number.isSafeInteger(anchor.line) || anchor.line < 1) return null;
  return { line: anchor.line, path, side: anchor.side };
}

function eventToPullRequestComment(event) {
  const labels = getAllTags(event, "t").map((label) => label.toLowerCase());
  const isReviewRequest = labels.includes(PR_REVIEW_REQUEST_LABEL);
  const isApproval = labels.includes(PR_APPROVAL_LABEL);
  const isChangeRequest = labels.includes(PR_CHANGES_REQUESTED_LABEL);
  const lineTag = getTag(event, "line");
  const parsedLine =
    lineTag && /^[1-9]\d*$/.test(lineTag) ? Number(lineTag) : Number.NaN;
  const anchor =
    isReviewRequest || isApproval || isChangeRequest
      ? null
      : normalizeProjectPullRequestCommentAnchor({
          line: parsedLine,
          path: getTag(event, "file"),
          side: getTag(event, "side"),
        });
  return {
    id: event.id,
    content: event.content,
    author: event.pubkey,
    createdAt: event.created_at,
    commit: getTag(event, "c") ?? null,
    anchor,
    isInlineComment:
      Boolean(anchor) || labels.includes(PR_INLINE_COMMENT_LABEL),
    isApproval,
    isChangeRequest,
    isReviewRequest,
    reviewDecision:
      isApproval === isChangeRequest
        ? null
        : isApproval
          ? "approved"
          : "changes-requested",
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

function reviewDecisionCommit(comment, initialCommit) {
  return comment.commit ?? initialCommit;
}

function trustedReviewActors(pullRequest, reviewers) {
  const author = pullRequest.pubkey.toLowerCase();
  const trustedActors = new Set(reviewers);
  for (const actor of allowedActorsForRoot(pullRequest)) {
    if (actor !== author) trustedActors.add(actor);
  }
  return trustedActors;
}

/** Latest trusted, current-commit review decision per author. */
function reviewDecisionsForPullRequest(
  comments,
  trustedActors,
  initialCommit,
  currentCommit,
) {
  const byAuthor = new Map();
  for (const comment of comments) {
    if (!comment.reviewDecision || !currentCommit) continue;
    const key = comment.author.toLowerCase();
    if (!trustedActors.has(key)) continue;
    const commit = reviewDecisionCommit(comment, initialCommit);
    if (commit !== currentCommit) continue;
    const existing = byAuthor.get(key);
    if (
      !existing ||
      comment.createdAt > existing.createdAt ||
      (comment.createdAt === existing.createdAt && comment.id > existing.id)
    ) {
      byAuthor.set(key, { ...comment, commit });
    }
  }
  const decisions = [...byAuthor.values()]
    .map(({ id, author: reviewer, createdAt, commit, reviewDecision }) => ({
      id,
      author: reviewer,
      createdAt,
      commit,
      reviewDecision,
    }))
    .sort(
      (left, right) =>
        left.createdAt - right.createdAt || left.id.localeCompare(right.id),
    );
  return {
    approvals: decisions.filter(
      (decision) => decision.reviewDecision === "approved",
    ),
    changeRequests: decisions.filter(
      (decision) => decision.reviewDecision === "changes-requested",
    ),
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
  const parsedComments = eventsForPullRequest(
    pullRequest.id,
    commentEvents,
  ).map(eventToPullRequestComment);
  const reviewers = reviewersForPullRequest(pullRequest, parsedComments);
  const trustedActors = trustedReviewActors(pullRequest, reviewers);
  const trustedReviewRequestActors = allowedActorsForRoot(pullRequest);
  const latestCommit = getTag(latestUpdate ?? pullRequest, "c") ?? null;
  const initialCommit = getTag(pullRequest, "c") ?? null;
  const comments = parsedComments.map((comment) => ({
    ...comment,
    inlineCommentStatus: comment.anchor
      ? latestCommit &&
        reviewDecisionCommit(comment, initialCommit) === latestCommit
        ? "current"
        : "outdated"
      : null,
    isTrustedReviewDecision:
      Boolean(comment.reviewDecision) &&
      trustedActors.has(comment.author.toLowerCase()),
    reviewDecisionStatus:
      comment.reviewDecision && trustedActors.has(comment.author.toLowerCase())
        ? latestCommit &&
          reviewDecisionCommit(comment, initialCommit) === latestCommit
          ? "current"
          : "historical"
        : null,
    isTrustedReviewRequest:
      comment.isReviewRequest &&
      trustedReviewRequestActors.has(comment.author.toLowerCase()),
  }));
  const title =
    getTag(pullRequest, "subject") ||
    pullRequest.content.split("\n")[0] ||
    "Untitled pull request";
  const reviewDecisions = reviewDecisionsForPullRequest(
    comments,
    trustedActors,
    initialCommit,
    latestCommit,
  );

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
    approvals: reviewDecisions.approvals,
    changeRequests: reviewDecisions.changeRequests,
    status: statusFromEvent(pullRequest, latestStatus),
    statusEventId: latestStatus?.id ?? null,
    statusCreatedAt: latestStatus?.created_at ?? null,
    branchName: getTag(pullRequest, "branch-name") ?? null,
    targetBranch: getTag(pullRequest, "target-branch") ?? null,
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
