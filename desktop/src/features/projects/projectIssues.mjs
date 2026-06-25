export const PROJECT_ISSUE_STATUS = {
  TRIAGE: "Triage",
  BACKLOG: "Backlog",
  IN_PROGRESS: "In Progress",
  IN_REVIEW: "In Review",
  DONE: "Done",
  CLOSED: "Closed",
};

export function getTag(event, name) {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

export function getAllTags(event, name) {
  return event.tags.filter((tag) => tag[0] === name).map((tag) => tag[1]);
}

function latestStatusForIssue(issueId, statusEvents) {
  return statusEvents
    .filter((event) =>
      event.tags.some((tag) => tag[0] === "e" && tag[1] === issueId),
    )
    .sort((left, right) => right.created_at - left.created_at)[0];
}

function statusFromEvent(issue, statusEvent) {
  if (statusEvent?.kind === 1631) return PROJECT_ISSUE_STATUS.DONE;
  if (statusEvent?.kind === 1632) return PROJECT_ISSUE_STATUS.CLOSED;
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

export function eventToProjectIssue(issue, statusEvents = []) {
  const latestStatus = latestStatusForIssue(issue.id, statusEvents);
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
    updatedAt: latestStatus?.created_at ?? issue.created_at,
  };
}

export function projectIssueEventsToIssues(issueEvents, statusEvents = []) {
  return [...issueEvents]
    .map((issue) => eventToProjectIssue(issue, statusEvents))
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
