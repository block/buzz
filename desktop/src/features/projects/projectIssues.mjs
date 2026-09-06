import { sortEvents } from "../../shared/api/relayClientShared.ts";

// Issue assignment mirrors PR review requests (projectPullRequests.mjs):
// a kind:1 comment labeled with this `t` tag whose `p` tags are the
// assignees. Labeled text notes stay readable for any client that treats
// them as plain comments, and the `p` tags route the assignment into the
// assignee's mention feed (inbox) for free.
export const ISSUE_ASSIGNMENT_LABEL = "assignment";
export const ISSUE_UNASSIGNMENT_LABEL = "unassignment";
export const ISSUE_ACTION_REQUIRED_LABEL = "action-required";
export const MYBUZZ_WORKFLOW_STATUS_LABEL = "mybuzz-workflow-status";
export const MYBUZZ_WORKFLOW_STATUS_WORKFLOW = "mybuzz-status-v1";
export const MYBUZZ_WORKFLOW_STATUS_OWNER =
  "dd57c78422bccf568feb2a7ae5bcf4d7ebefc2c6c54bf56a26faeb9e0b08d36b";

export const MYBUZZ_WORKFLOW_STATUS_STATES = [
  "triage",
  "backlog",
  "in-development",
  "implemented",
  "code-qs",
  "to-be-published",
  "ready-for-test",
];

export const MYBUZZ_WORKFLOW_STATUS_LABELS = {
  triage: "Triage",
  backlog: "Backlog",
  "in-development": "In Development",
  implemented: "Implemented",
  "code-qs": "Code-QS",
  "to-be-published": "To Be Published",
  "ready-for-test": "Ready for Test",
};

const MYBUZZ_WORKFLOW_STATUS_REASON_REQUIRED = new Set([
  "triage",
  "backlog",
  "ready-for-test",
]);

export function isHumanDirectedIssueComment(body) {
  return /^(?:test|expected|reply)\s*:/i.test(body.trimStart());
}

export const PROJECT_ISSUE_STATUS = {
  TRIAGE: "Triage",
  BACKLOG: "Backlog",
  IN_PROGRESS: "In Progress",
  APPROVED: "Approved",
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
  return sortEvents(issueCommentEvents)
    .filter((event) => !isWorkflowActivityEvent(event))
    .map((event) => ({
      id: event.id,
      content: event.content,
      tags: getImetaTags(event),
      author: event.pubkey,
      createdAt: event.created_at,
      recipients: getAllTags(event, "p"),
      actionRequired: getAllTags(event, "t").includes(
        ISSUE_ACTION_REQUIRED_LABEL,
      ),
    }));
}

function isWorkflowActivityEvent(event) {
  const labels = getAllTags(event, "t");
  return (
    isMyBuzzWorkflowStatusEvent(event) ||
    labels.includes(ISSUE_ASSIGNMENT_LABEL) ||
    labels.includes(ISSUE_UNASSIGNMENT_LABEL) ||
    labels.includes("test-ready") ||
    labels.includes("review-ready") ||
    labels.includes("technical-review") ||
    labels.includes("release-published") ||
    labels.includes("human-verdict") ||
    labels.includes("issue-verdict-confirmed")
  );
}

function isLowercaseHex64(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function isPrintableReason(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.trim() === value &&
    !hasControlCharacters(value)
  );
}

function exactWorkflowStatusTags(event, issueId, repoAddress) {
  const knownTags = new Set(["e", "a", "t", "workflow", "state", "reason"]);
  if (
    !Array.isArray(event.tags) ||
    event.tags.some((tag) => !Array.isArray(tag) || !knownTags.has(tag[0]))
  ) {
    return null;
  }
  const exactOne = (name, value) =>
    event.tags.filter((tag) => tag[0] === name).length === 1 &&
    exactTag(event, name, value);
  if (
    !exactOne("e", [issueId, "", "root"]) ||
    !exactOne("a", [repoAddress]) ||
    !exactOne("t", [MYBUZZ_WORKFLOW_STATUS_LABEL]) ||
    !exactOne("workflow", [MYBUZZ_WORKFLOW_STATUS_WORKFLOW])
  ) {
    return null;
  }
  const state = singleTagValue(event, "state");
  if (!state || !MYBUZZ_WORKFLOW_STATUS_STATES.includes(state)) return null;
  const reasons = event.tags.filter((tag) => tag[0] === "reason");
  if (reasons.length > 1) return null;
  const reason = reasons.length === 1 ? singleTagValue(event, "reason") : null;
  if (reasons.length === 1 && !reason) return null;
  if (
    (MYBUZZ_WORKFLOW_STATUS_REASON_REQUIRED.has(state) &&
      !isPrintableReason(reason)) ||
    (reason !== null && !isPrintableReason(reason))
  ) {
    return null;
  }
  return { reason, state };
}

function isMyBuzzWorkflowStatusEvent(event) {
  const issueId = getTag(event, "e");
  const repoAddress = getTag(event, "a");
  return (
    Boolean(issueId) &&
    Boolean(repoAddress) &&
    exactWorkflowStatusTags(event, issueId, repoAddress) !== null
  );
}

function workflowStatusesForIssue(issue, issueCommentEvents) {
  const repoAddress = getTag(issue, "a");
  if (!repoAddress) return [];
  const byId = new Map();
  for (const event of issueCommentEvents) {
    if (
      event.kind !== 1 ||
      event.pubkey.toLowerCase() !== MYBUZZ_WORKFLOW_STATUS_OWNER ||
      !isLowercaseHex64(event.id) ||
      !isLowercaseHex64(event.pubkey) ||
      !Number.isInteger(event.created_at)
    ) {
      continue;
    }
    const parsed = exactWorkflowStatusTags(event, issue.id, repoAddress);
    if (!parsed || byId.has(event.id)) continue;
    byId.set(event.id, { event, ...parsed });
  }
  return [...byId.values()].sort(
    (left, right) =>
      right.event.created_at - left.event.created_at ||
      right.event.id.localeCompare(left.event.id),
  );
}

function activityForIssue(issue, issueCommentEvents, workflowStatuses) {
  const repoAddress = getTag(issue, "a");
  const trustedActors = allowedActorsForRoot(issue);
  const activity = workflowStatuses.map(({ event, reason, state }) => ({
    id: event.id,
    createdAt: event.created_at,
    text: `Workflow status: ${MYBUZZ_WORKFLOW_STATUS_LABELS[state]}${reason ? ` - ${reason}` : ""}`,
  }));
  if (workflowStatuses.length === 0) {
    activity.push({
      id: "not-yet-classified",
      createdAt: issue.created_at,
      text: "Not yet classified",
    });
  }
  for (const event of issueCommentEvents) {
    const labels = getAllTags(event, "t");
    if (
      event.kind !== 1 ||
      !repoAddress ||
      !exactlyOneTag(event, "e", [issue.id, "", "root"]) ||
      !exactlyOneTag(event, "a", [repoAddress])
    ) {
      continue;
    }
    if (
      (labels.includes(ISSUE_ASSIGNMENT_LABEL) ||
        labels.includes(ISSUE_UNASSIGNMENT_LABEL)) &&
      trustedActors.has(event.pubkey.toLowerCase())
    ) {
      for (const pubkey of getAllTags(event, "p")) {
        activity.push({
          id: `${event.id}:${pubkey}`,
          createdAt: event.created_at,
          text: `${labels.includes(ISSUE_ASSIGNMENT_LABEL) ? "Assigned" : "Unassigned"}: ${pubkey.slice(0, 12)}...`,
        });
      }
      continue;
    }
    const milestone = [
      "test-ready",
      "review-ready",
      "technical-review",
      "release-published",
      "human-verdict",
    ].find((label) => labels.includes(label));
    if (milestone && trustedActors.has(event.pubkey.toLowerCase())) {
      activity.push({
        id: event.id,
        createdAt: event.created_at,
        text: `Technical milestone: ${milestone}`,
      });
    }
  }
  return activity.sort(
    (left, right) =>
      right.createdAt - left.createdAt || right.id.localeCompare(left.id),
  );
}

function exactTag(event, name, value) {
  return event.tags.some(
    (tag) =>
      tag.length === value.length + 1 &&
      tag[0] === name &&
      value.every((part, index) => tag[index + 1] === part),
  );
}

function exactlyOneTag(event, name, value) {
  return (
    event.tags.filter((tag) => tag[0] === name).length === 1 &&
    exactTag(event, name, value)
  );
}

function singleTagValue(event, name) {
  const tags = event.tags.filter((tag) => tag[0] === name);
  return tags.length === 1 &&
    tags[0].length === 2 &&
    isNonEmptyString(tags[0][1])
    ? tags[0][1]
    : null;
}

function normalizedConfiguredPubkeys(pubkeys) {
  if (!Array.isArray(pubkeys) || pubkeys.length === 0) return [];
  if (pubkeys.some((pubkey) => !/^[0-9a-f]{64}$/.test(pubkey))) return [];
  return new Set(pubkeys).size === pubkeys.length ? [...pubkeys] : [];
}

function exactContentFields(content, header, expectedNames) {
  const lines = content.split("\n");
  if (lines[0] !== header || lines.length !== expectedNames.length + 1) {
    return null;
  }
  const fields = new Map();
  for (const line of lines.slice(1)) {
    const separator = line.indexOf(": ");
    if (separator <= 0) return null;
    const name = line.slice(0, separator);
    const value = line.slice(separator + 2);
    if (!expectedNames.includes(name) || !value || fields.has(name))
      return null;
    fields.set(name, value);
  }
  return fields.size === expectedNames.length ? fields : null;
}

function reviewContent(event, reviewId) {
  const fields = exactContentFields(event.content, "[REVIEW-READY]", [
    "Review-ID",
    "Target",
    "Evidence",
    "Test",
    "Known limitations",
  ]);
  if (!fields || fields.get("Review-ID") !== reviewId) return null;
  return {
    target: fields.get("Target"),
    evidence: fields.get("Evidence"),
    test: fields.get("Test"),
    limitations: fields.get("Known limitations"),
  };
}

function exactRecipientSet(event, expectedRecipients) {
  const recipientTags = event.tags.filter((tag) => tag[0] === "p");
  if (recipientTags.length !== expectedRecipients.size) return false;
  const recipients = recipientTags.map((tag) =>
    tag.length === 2 && /^[0-9a-f]{64}$/.test(tag[1] ?? "") ? tag[1] : null,
  );
  return (
    recipients.every(Boolean) &&
    new Set(recipients).size === recipients.length &&
    recipients.every((recipient) => expectedRecipients.has(recipient))
  );
}

function descendingEventOrder(left, right) {
  return right.created_at - left.created_at || right.id.localeCompare(left.id);
}

function currentReviewForIssue(issue, issueCommentEvents, reviewAuthority) {
  const repoAddress = getTag(issue, "a");
  const coordinators = normalizedConfiguredPubkeys(
    reviewAuthority?.coordinatorPubkeys,
  );
  const humans = normalizedConfiguredPubkeys(reviewAuthority?.humanPubkeys);
  if (
    !repoAddress ||
    !repoOwnerFromAddress(repoAddress) ||
    coordinators.length === 0 ||
    humans.length !== 2
  ) {
    return null;
  }
  const expectedHumans = new Set(humans);
  const candidates = issueCommentEvents
    .filter(
      (event) =>
        event.kind === 1 &&
        coordinators.includes(event.pubkey) &&
        event.tags.some((tag) => tag[0] === "t" && tag[1] === "review-ready"),
    )
    .sort(descendingEventOrder);
  const marker = candidates[0];
  if (
    !marker ||
    !/^[0-9a-f]{64}$/.test(marker.id) ||
    marker.created_at < issue.created_at
  ) {
    return null;
  }
  if (
    !exactlyOneTag(marker, "e", [issue.id, "", "root"]) ||
    !exactlyOneTag(marker, "a", [repoAddress]) ||
    !exactlyOneTag(marker, "t", ["review-ready"]) ||
    getAllTags(marker, "review-root").length > 0 ||
    !exactRecipientSet(marker, expectedHumans)
  ) {
    return null;
  }
  const id = singleTagValue(marker, "review");
  const content = id ? reviewContent(marker, id) : null;
  if (!id || !/^[0-9a-f]{64}$/.test(id) || !content) {
    return null;
  }
  if (
    candidates.some(
      (candidate, index) =>
        index > 0 && singleTagValue(candidate, "review") === id,
    )
  ) {
    return null;
  }
  return {
    marker,
    coordinators,
    review: {
      id,
      ...content,
      authorizedHumanPubkeys: humans,
      verdict: null,
    },
  };
}

function hasControlCharacters(value) {
  return [...value].some((character) => {
    const code = character.charCodeAt(0);
    return code <= 31 || code === 127;
  });
}

function humanVerdictForCurrentReview(
  issue,
  statusEvents,
  currentReviewBinding,
) {
  if (!currentReviewBinding) return null;
  const { marker, review } = currentReviewBinding;
  const repoAddress = getTag(issue, "a");
  const authorizedHumans = new Set(review.authorizedHumanPubkeys);
  const expectedRecipients = new Set(
    [repoOwnerFromAddress(repoAddress), issue.pubkey.toLowerCase()].filter(
      Boolean,
    ),
  );
  return (
    statusEvents
      .filter((event) => {
        const verdict = singleTagValue(event, "verdict");
        if (
          !/^[0-9a-f]{64}$/.test(event.id) ||
          !authorizedHumans.has(event.pubkey) ||
          event.created_at < marker.created_at ||
          !exactlyOneTag(event, "e", [issue.id, "", "root"]) ||
          !exactlyOneTag(event, "a", [repoAddress]) ||
          !exactlyOneTag(event, "t", ["human-verdict"]) ||
          !exactlyOneTag(event, "review", [review.id]) ||
          getAllTags(event, "review-root").length > 0 ||
          !exactRecipientSet(event, expectedRecipients)
        ) {
          return false;
        }
        if (event.kind === 1631 && verdict === "accepted") {
          return event.content === "";
        }
        return (
          event.kind === 1630 &&
          verdict === "rejected" &&
          event.content.length > 0 &&
          event.content.length <= 500 &&
          event.content.trim() === event.content &&
          !hasControlCharacters(event.content)
        );
      })
      .sort(
        (left, right) =>
          left.created_at - right.created_at || left.id.localeCompare(right.id),
      )[0] ?? null
  );
}

function confirmationContent(event, issue, review, rawVerdict, kanbanStatus) {
  const verdict = singleTagValue(rawVerdict, "verdict");
  const expectedNames = [
    "Issue",
    "Repository",
    "Review",
    "Verdict-Event",
    "Actor",
    "Verdict",
    "Kanban-Status",
    ...(verdict === "rejected" ? ["Reason"] : []),
  ];
  const fields = exactContentFields(
    event.content,
    "[ISSUE-VERDICT-CONFIRMED]",
    expectedNames,
  );
  if (!fields) return false;
  return (
    fields.get("Issue") === issue.id &&
    fields.get("Repository") === getTag(issue, "a") &&
    fields.get("Review") === review.id &&
    fields.get("Verdict-Event") === rawVerdict.id &&
    fields.get("Actor") === rawVerdict.pubkey &&
    fields.get("Verdict") === verdict &&
    fields.get("Kanban-Status") === kanbanStatus &&
    (verdict === "accepted"
      ? !fields.has("Reason")
      : fields.get("Reason") === rawVerdict.content)
  );
}

function confirmationForCurrentVerdict(
  issue,
  issueCommentEvents,
  currentReviewBinding,
  rawVerdict,
) {
  if (!currentReviewBinding || !rawVerdict) return null;
  const { coordinators, marker, review } = currentReviewBinding;
  const verdict = singleTagValue(rawVerdict, "verdict");
  const allowedKanbanStatuses =
    verdict === "accepted" ? new Set(["done"]) : new Set(["ready", "todo"]);
  const candidates = issueCommentEvents.filter(
    (event) =>
      event.kind === 1 &&
      coordinators.includes(event.pubkey) &&
      event.created_at >= marker.created_at &&
      event.tags.some(
        (tag) => tag[0] === "t" && tag[1] === "issue-verdict-confirmed",
      ),
  );
  if (candidates.length !== 1) return null;
  const confirmations = candidates.filter((event) => {
    if (
      !/^[0-9a-f]{64}$/.test(event.id) ||
      event.created_at < rawVerdict.created_at ||
      !exactlyOneTag(event, "e", [issue.id, "", "root"]) ||
      !exactlyOneTag(event, "a", [getTag(issue, "a")]) ||
      !exactlyOneTag(event, "t", ["issue-verdict-confirmed"]) ||
      !exactlyOneTag(event, "review", [review.id]) ||
      getAllTags(event, "review-root").length > 0 ||
      !exactlyOneTag(event, "verdict", [verdict]) ||
      !exactlyOneTag(event, "verdict-event", [rawVerdict.id]) ||
      !exactRecipientSet(event, new Set([rawVerdict.pubkey]))
    ) {
      return false;
    }
    const kanbanStatus = singleTagValue(event, "kanban-status");
    return (
      allowedKanbanStatuses.has(kanbanStatus) &&
      confirmationContent(event, issue, review, rawVerdict, kanbanStatus)
    );
  });
  if (confirmations.length !== 1) return null;
  const event = confirmations[0];
  return {
    event,
    model: {
      eventId: event.id,
      createdAt: event.created_at,
      kanbanStatus: singleTagValue(event, "kanban-status"),
    },
  };
}

export function eventToProjectIssue(
  issue,
  statusEvents = [],
  commentEvents = [],
  _additionalStatusActors = [],
  reviewAuthority,
) {
  const issueCommentEvents = commentEvents.filter((event) =>
    event.tags.some(
      (tag) => (tag[0] === "e" || tag[0] === "E") && tag[1] === issue.id,
    ),
  );
  const assignmentState = assignmentStateForIssue(issue, issueCommentEvents);
  const currentReviewBinding = currentReviewForIssue(
    issue,
    issueCommentEvents,
    reviewAuthority,
  );
  const rawVerdict = humanVerdictForCurrentReview(
    issue,
    statusEvents,
    currentReviewBinding,
  );
  const confirmation = confirmationForCurrentVerdict(
    issue,
    issueCommentEvents,
    currentReviewBinding,
    rawVerdict,
  );
  const currentReview = currentReviewBinding
    ? {
        ...currentReviewBinding.review,
        verdict: rawVerdict
          ? {
              eventId: rawVerdict.id,
              kind: singleTagValue(rawVerdict, "verdict"),
              actorPubkey: rawVerdict.pubkey,
              createdAt: rawVerdict.created_at,
              reason: rawVerdict.kind === 1630 ? rawVerdict.content : null,
              confirmation: confirmation?.model ?? null,
            }
          : null,
      }
    : null;
  const directVerdict = currentReview?.verdict?.kind ?? null;
  const workflowStatuses = workflowStatusesForIssue(issue, issueCommentEvents);
  const workflowStatus = workflowStatuses[0] ?? null;
  const status =
    directVerdict === "accepted"
      ? PROJECT_ISSUE_STATUS.DONE
      : workflowStatus
        ? MYBUZZ_WORKFLOW_STATUS_LABELS[workflowStatus.state]
        : PROJECT_ISSUE_STATUS.TRIAGE;
  const effectiveStatus = rawVerdict ?? workflowStatus?.event ?? null;
  const comments = commentsForIssue(issueCommentEvents);
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
    status,
    workflowStatus: workflowStatus
      ? {
          eventId: workflowStatus.event.id,
          state: workflowStatus.state,
          reason: workflowStatus.reason,
          createdAt: workflowStatus.event.created_at,
        }
      : null,
    activity: activityForIssue(issue, issueCommentEvents, workflowStatuses),
    statusEventId: effectiveStatus?.id ?? null,
    statusCreatedAt: effectiveStatus?.created_at ?? null,
    updatedAt:
      [
        ...comments,
        ...(effectiveStatus ? [{ createdAt: effectiveStatus.created_at }] : []),
      ].sort((left, right) => right.createdAt - left.createdAt)[0]?.createdAt ??
      issue.created_at,
    currentReview,
    comments,
  };
}

export function projectIssueEventsToIssues(
  issueEvents,
  statusEvents = [],
  commentEvents = [],
  additionalStatusActors = [],
  reviewAuthority,
) {
  return [...issueEvents]
    .map((issue) =>
      eventToProjectIssue(
        issue,
        statusEvents,
        commentEvents,
        additionalStatusActors,
        reviewAuthority,
      ),
    )
    .sort((left, right) => right.updatedAt - left.updatedAt);
}

/** Keep consecutive status changes ordered across whole-second Nostr timestamps. */
export function nextProjectIssueStatusCreatedAt(issue, now) {
  return Math.max(now, (issue.statusCreatedAt ?? 0) + 1);
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
