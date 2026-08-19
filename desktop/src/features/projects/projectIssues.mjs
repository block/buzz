import { sortEvents } from "../../shared/api/relayClientShared.ts";

// Issue assignment mirrors PR review requests (projectPullRequests.mjs):
// a kind:1 comment labeled with this `t` tag whose `p` tags are the
// assignees. Labeled text notes stay readable for any client that treats
// them as plain comments, and the `p` tags route the assignment into the
// assignee's mention feed (inbox) for free.
export const ISSUE_ASSIGNMENT_LABEL = "assignment";
export const ISSUE_UNASSIGNMENT_LABEL = "unassignment";

export const PROJECT_ISSUE_STATUS = {
  TRIAGE: "Triage",
  BACKLOG: "Backlog",
  IN_PROGRESS: "In Progress",
  IN_REVIEW: "In Review",
  DONE: "Done",
  CLOSED: "Closed",
};

function isNonEmptyString(value) {
  return typeof value === "string" && value.length > 0;
}

export function getTag(event, name) {
  const value = event.tags.find((tag) => tag[0] === name)?.[1];
  return isNonEmptyString(value) ? value : undefined;
}

export function getAllTags(event, name) {
  return event.tags
    .filter((tag) => tag[0] === name && isNonEmptyString(tag[1]))
    .map((tag) => tag[1]);
}

export function getImetaTags(event) {
  return event.tags.filter((tag) => tag[0] === "imeta");
}

function repoOwnerFromAddress(repoAddress) {
  const owner = (repoAddress ?? "").split(":")[1] ?? "";
  return /^[a-fA-F0-9]{64}$/.test(owner) ? owner.toLowerCase() : null;
}

/**
 * Pubkeys allowed to change a root event's lifecycle (status, updates):
 * the root author and the owner of the repo the root event targets.
 * Anyone else's status/update events are ignored (NIP-34 scopes these
 * to the root author or a maintainer).
 */
export function allowedActorsForRoot(rootEvent) {
  const allowed = new Set([rootEvent.pubkey.toLowerCase()]);
  const owner = repoOwnerFromAddress(getTag(rootEvent, "a"));
  if (owner) allowed.add(owner);
  return allowed;
}

function latestStatusForIssue(issue, statusEvents) {
  const allowedActors = allowedActorsForRoot(issue);
  return statusEvents
    .filter(
      (event) =>
        allowedActors.has(event.pubkey.toLowerCase()) &&
        event.tags.some((tag) => tag[0] === "e" && tag[1] === issue.id),
    )
    .sort((left, right) => right.created_at - left.created_at)[0];
}

function statusFromEvent(issue, statusEvent) {
  if (statusEvent?.kind === 1631) return PROJECT_ISSUE_STATUS.DONE;
  if (statusEvent?.kind === 1632) return PROJECT_ISSUE_STATUS.CLOSED;
  // NIP-34 calls 1633 "Draft"; we surface it as Triage for issues. The
  // label-based fallbacks below are client-side heuristics, not protocol.
  if (statusEvent?.kind === 1633) return PROJECT_ISSUE_STATUS.TRIAGE;

  const labels = getAllTags(issue, "t").map((label) => label.toLowerCase());
  if (labels.includes("in-review") || labels.includes("review")) {
    return PROJECT_ISSUE_STATUS.IN_REVIEW;
  }
  if (labels.includes("in-progress") || labels.includes("active")) {
    return PROJECT_ISSUE_STATUS.IN_PROGRESS;
  }
  if (labels.includes("triage")) return PROJECT_ISSUE_STATUS.TRIAGE;
  return PROJECT_ISSUE_STATUS.BACKLOG;
}

/**
 * Assignment state is reduced from trusted kind:1 operations. `t: assignment`
 * adds each `p` tag and `t: unassignment` removes it. The issue root's `p`
 * tags are notification routing only.
 *
 * Trusted signers are the issue author and repo owner (who may change anyone),
 * plus any community member whose operation names only themselves. Uncaused
 * self-service operations are applied first, authoritative operations second,
 * and self-service operations that causally reference the current per-assignee
 * operation head last. This prevents signer-controlled timestamps from
 * overriding authority while allowing a later observed owner/author decision
 * to be superseded by the affected assignee.
 */
function assignmentStateForIssue(issue, issueCommentEvents) {
  const allowedActors = allowedActorsForRoot(issue);
  const assignees = new Set();
  const operationHeads = new Map();
  const uncausedSelfServiceOperations = [];
  const authoritativeOperations = [];
  const causalSelfServiceOperations = [];
  const events = sortEvents(
    issueCommentEvents.filter(
      (event) =>
        event.kind === 1 &&
        event.tags.some((tag) => tag[0] === "e" && tag[1] === issue.id),
    ),
  );
  for (const event of events) {
    const labels = getAllTags(event, "t");
    const isAssignment = labels.includes(ISSUE_ASSIGNMENT_LABEL);
    const isUnassignment = labels.includes(ISSUE_UNASSIGNMENT_LABEL);
    if (isAssignment === isUnassignment) continue;
    const signer = event.pubkey.toLowerCase();
    const pubkeys = getAllTags(event, "p").map((pubkey) =>
      pubkey.toLowerCase(),
    );
    const isSelfOperation = pubkeys.length === 1 && pubkeys[0] === signer;
    if (!allowedActors.has(signer) && !isSelfOperation) continue;
    const operation = {
      id: event.id.toLowerCase(),
      isAssignment,
      pubkeys,
    };
    if (allowedActors.has(signer)) {
      authoritativeOperations.push(operation);
    } else {
      const priorTags = event.tags.filter((tag) => tag[0] === "prior");
      if (priorTags.length === 0) {
        uncausedSelfServiceOperations.push(operation);
        continue;
      }
      if (
        priorTags.length !== 1 ||
        !/^[a-fA-F0-9]{64}$/.test(priorTags[0]?.[1] ?? "")
      ) {
        continue;
      }
      causalSelfServiceOperations.push({
        ...operation,
        prior: priorTags[0][1].toLowerCase(),
      });
    }
  }
  for (const { id, isAssignment, pubkeys, prior } of [
    ...uncausedSelfServiceOperations,
    ...authoritativeOperations,
    ...causalSelfServiceOperations,
  ]) {
    if (prior && operationHeads.get(pubkeys[0]) !== prior) continue;
    for (const pubkey of pubkeys) {
      if (isAssignment) {
        assignees.add(pubkey);
      } else {
        assignees.delete(pubkey);
      }
      operationHeads.set(pubkey, id);
    }
  }
  return {
    assignees: [...assignees],
    heads: Object.fromEntries(operationHeads),
  };
}

function commentsForIssue(issueCommentEvents) {
  return sortEvents(issueCommentEvents).map((event) => ({
    id: event.id,
    content: event.content,
    tags: getImetaTags(event),
    author: event.pubkey,
    createdAt: event.created_at,
  }));
}

export function eventToProjectIssue(
  issue,
  statusEvents = [],
  commentEvents = [],
) {
  const latestStatus = latestStatusForIssue(issue, statusEvents);
  const issueCommentEvents = commentEvents.filter((event) =>
    event.tags.some(
      (tag) => (tag[0] === "e" || tag[0] === "E") && tag[1] === issue.id,
    ),
  );
  const comments = commentsForIssue(issueCommentEvents);
  const assignmentState = assignmentStateForIssue(issue, issueCommentEvents);
  const title =
    getTag(issue, "subject") ||
    issue.content.split("\n")[0] ||
    "Untitled issue";

  return {
    id: issue.id,
    title,
    content: issue.content,
    tags: getImetaTags(issue),
    author: issue.pubkey,
    createdAt: issue.created_at,
    repoAddress: getTag(issue, "a") ?? null,
    channelId: getTag(issue, "h") ?? null,
    originAgentName: getTag(issue, "buzz-origin-agent") ?? null,
    labels: getAllTags(issue, "t"),
    recipients: getAllTags(issue, "p"),
    assignees: assignmentState.assignees,
    assigneeOperationHeads: assignmentState.heads,
    status: statusFromEvent(issue, latestStatus),
    statusEventId: latestStatus?.id ?? null,
    updatedAt:
      [
        ...comments,
        ...(latestStatus ? [{ createdAt: latestStatus.created_at }] : []),
      ].sort((left, right) => right.createdAt - left.createdAt)[0]?.createdAt ??
      issue.created_at,
    comments,
  };
}

export function projectIssueEventsToIssues(
  issueEvents,
  statusEvents = [],
  commentEvents = [],
) {
  return [...issueEvents]
    .map((issue) => eventToProjectIssue(issue, statusEvents, commentEvents))
    .sort((left, right) => right.updatedAt - left.updatedAt);
}

/** Keep consecutive comments ordered across whole-second Nostr timestamps. */
export function nextProjectIssueCommentCreatedAt(issue, now, author) {
  const normalizedAuthor = author.toLowerCase();
  return Math.max(
    now,
    ...issue.comments
      .filter((comment) => comment.author.toLowerCase() === normalizedAuthor)
      .map((comment) => comment.createdAt + 1),
  );
}

export function buildGitIssueTags({
  repoAddress,
  repoOwner,
  title,
  labels = [],
}) {
  if (!repoAddress.startsWith("30617:")) {
    throw new Error("Issue repo address must reference a kind:30617 repo.");
  }
  if (!/^[a-fA-F0-9]{64}$/.test(repoOwner)) {
    throw new Error("Repo owner must be 64 hex characters.");
  }
  const subject = title.trim();
  if (!subject) {
    throw new Error("Issue title is required.");
  }
  if (subject.length > 256) {
    throw new Error("Issue title must be 256 characters or fewer.");
  }

  const tags = [
    ["a", repoAddress],
    ["p", repoOwner.toLowerCase()],
    ["subject", subject],
  ];

  for (const label of labels) {
    const trimmed = label.trim();
    if (trimmed) tags.push(["t", trimmed]);
  }

  return tags;
}

export function buildGitStatusTags({ issueId, repoAddress, repoOwner }) {
  if (!/^[a-fA-F0-9]{64}$/.test(issueId)) {
    throw new Error("Issue ID must be 64 hex characters.");
  }
  const tags = [["e", issueId, "", "root"]];
  if (repoAddress) tags.push(["a", repoAddress]);
  if (repoOwner && /^[a-fA-F0-9]{64}$/.test(repoOwner)) {
    tags.push(["p", repoOwner.toLowerCase()]);
  }
  return tags;
}
