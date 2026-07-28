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

function isCanonicalRepoAddress(repoAddress) {
  const match = /^30617:[a-f0-9]{64}:([A-Za-z0-9._-]{1,64})$/.exec(repoAddress);
  const repoId = match?.[1] ?? "";
  return Boolean(match && !repoId.startsWith(".") && !repoId.includes(".."));
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

function parseIssueAssignmentEvent(event) {
  const pTags = event.tags.filter((tag) => tag[0] === "p");
  const markedAssignees = event.tags.filter(
    (tag) =>
      tag.length === 4 &&
      tag[0] === "p" &&
      tag[2] === "" &&
      tag[3] === "assignee" &&
      /^[a-f0-9]{64}$/.test(tag[1] ?? ""),
  );
  const unassignTags = event.tags.filter(
    (tag) => tag.length === 2 && tag[0] === "assignee" && tag[1] === "none",
  );
  const assigneeTags = event.tags.filter((tag) => tag[0] === "assignee");

  if (
    pTags.length === 1 &&
    markedAssignees.length === 1 &&
    assigneeTags.length === 0
  ) {
    return { assignee: markedAssignees[0][1] };
  }
  if (
    pTags.length === 0 &&
    markedAssignees.length === 0 &&
    assigneeTags.length === 1 &&
    unassignTags.length === 1
  ) {
    return { assignee: null };
  }
  return null;
}

function isCanonicalIssueAssignmentEvent(issue, event, repoOwner) {
  const dTags = event.tags.filter((tag) => tag[0] === "d");
  const eTags = event.tags.filter((tag) => tag[0] === "e");
  const aTags = event.tags.filter((tag) => tag[0] === "a");
  return (
    event.kind === 32001 &&
    event.content === "" &&
    event.pubkey === repoOwner &&
    !event.tags.some((tag) => tag[0] === "h") &&
    dTags.length === 1 &&
    dTags[0].length === 2 &&
    dTags[0][1] === issue.id &&
    eTags.length === 1 &&
    eTags[0].length === 4 &&
    eTags[0][1] === issue.id &&
    /^[a-f0-9]{64}$/.test(eTags[0][1] ?? "") &&
    eTags[0][2] === "" &&
    eTags[0][3] === "root" &&
    aTags.length === 1 &&
    aTags[0].length === 2 &&
    aTags[0][1] === getTag(issue, "a") &&
    isCanonicalRepoAddress(aTags[0][1] ?? "") &&
    parseIssueAssignmentEvent(event) !== null
  );
}

function latestAssigneeForIssue(issue, assigneeEvents) {
  const repoOwner = repoOwnerFromAddress(getTag(issue, "a"));
  if (!repoOwner) return undefined;

  return assigneeEvents
    .filter((event) => isCanonicalIssueAssignmentEvent(issue, event, repoOwner))
    .sort(
      (left, right) =>
        right.created_at - left.created_at || left.id.localeCompare(right.id),
    )[0];
}

/**
 * Assignment is routing metadata, not acceptance or execution. Reassignment
 * does not cancel work already in flight; job lifecycle remains represented
 * separately by kinds 43001–43006. Only the repository owner can set this
 * shared routing state; issue authorship alone does not grant that authority.
 * See VISION_PROJECTS.md.
 */
function assigneeFromEvent(assigneeEvent) {
  if (!assigneeEvent) return undefined;
  return parseIssueAssignmentEvent(assigneeEvent)?.assignee;
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
      tags: getImetaTags(event),
      author: event.pubkey,
      createdAt: event.created_at,
    }));
}

export function eventToProjectIssue(
  issue,
  statusEvents = [],
  commentEvents = [],
  assigneeEvents = [],
) {
  const latestStatus = latestStatusForIssue(issue, statusEvents);
  const latestAssignee = latestAssigneeForIssue(issue, assigneeEvents);
  const comments = commentsForIssue(issue.id, commentEvents);
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
    status: statusFromEvent(issue, latestStatus),
    statusEventId: latestStatus?.id ?? null,
    assignee: assigneeFromEvent(latestAssignee) ?? null,
    assigneeEventId: latestAssignee?.id ?? null,
    assignedBy: latestAssignee?.pubkey ?? null,
    updatedAt:
      [
        ...comments,
        ...(latestStatus ? [{ createdAt: latestStatus.created_at }] : []),
        ...(latestAssignee ? [{ createdAt: latestAssignee.created_at }] : []),
      ].sort((left, right) => right.createdAt - left.createdAt)[0]?.createdAt ??
      issue.created_at,
    comments,
  };
}

export function projectIssueEventsToIssues(
  issueEvents,
  statusEvents = [],
  commentEvents = [],
  assigneeEvents = [],
) {
  return [...issueEvents]
    .map((issue) =>
      eventToProjectIssue(issue, statusEvents, commentEvents, assigneeEvents),
    )
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
