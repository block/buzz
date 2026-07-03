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

function commentsForIssue(issueId, commentEvents) {
  return commentEvents
    .filter((event) =>
      event.tags.some(
        (tag) => (tag[0] === "e" || tag[0] === "E") && tag[1] === issueId,
      ),
    )
    .sort((left, right) => left.created_at - right.created_at)
    .map((event) => ({
      id: event.id,
      content: event.content,
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
  const comments = commentsForIssue(issue.id, commentEvents);
  const title =
    getTag(issue, "subject") ||
    issue.content.split("\n")[0] ||
    "Untitled issue";

  return {
    id: issue.id,
    title,
    content: issue.content,
    author: issue.pubkey,
    createdAt: issue.created_at,
    repoAddress: getTag(issue, "a") ?? null,
    labels: getAllTags(issue, "t"),
    recipients: getAllTags(issue, "p"),
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
