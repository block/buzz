"use strict";

const MARKER = "<!-- codex-security-review -->";
const STALE_MARKER = "<!-- codex-security-review-stale -->";
const REVIEW_COMMAND = "@buzz-security-review";
const CURRENT_REVIEW_LABEL = "codex-security-review-current";
const RECONCILIATION_BATCH_SIZE = 32;
const MAX_RECONCILIATION_PASSES = 2;
const GITHUB_RETRY_ATTEMPTS = 3;
const RISKS = ["NONE", "LOW", "MEDIUM", "HIGH", "CRITICAL"];
const SEVERITIES = new Set(RISKS.slice(1));
const CATEGORIES = new Set([
  "Isolation",
  "Auth",
  "Event Integrity",
  "Cryptography",
  "Injection",
  "Agent/Workflow",
  "Desktop/Mobile",
  "Concurrency",
  "Reliability",
  "Supply Chain",
  "Other",
]);

const completedMarker = (baseSha, headSha) =>
  `<!-- codex-security-review-range:${baseSha}...${headSha} -->`;

const reviewCommand = (headSha) => `${REVIEW_COMMAND} ${headSha}`;

const isOrganizationMember = (association) =>
  association === "MEMBER" || association === "OWNER";

const hasCurrentReviewLabel = (pullRequest) =>
  pullRequest.labels?.some(
    (label) =>
      (typeof label === "string" ? label : label?.name) ===
      CURRENT_REVIEW_LABEL,
  ) ?? false;

const isObject = (value) =>
  value !== null && typeof value === "object" && !Array.isArray(value);

function requireKeys(value, expected, label) {
  if (!isObject(value)) {
    throw new Error(`${label} must be an object.`);
  }
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    throw new Error(`${label} has unexpected or missing properties.`);
  }
}

function requireString(value, maxLength, label) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > maxLength
  ) {
    throw new Error(
      `${label} must be a non-empty string of at most ${maxLength} characters.`,
    );
  }
  return value;
}

function safeCodeText(value, maxLength, label) {
  const input = requireString(value, maxLength, label)
    .replace(
      /[\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u2028-\u202e\u2066-\u2069]/g,
      " ",
    )
    .trim();
  if (!input) {
    throw new Error(`${label} is empty after removing control characters.`);
  }

  const longestBacktickRun = Math.max(
    0,
    ...(input.match(/`+/g) || []).map((run) => run.length),
  );
  const fence = "`".repeat(longestBacktickRun + 1);
  return `${fence} ${input} ${fence}`;
}

function validPath(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 500 &&
    !value.startsWith("/") &&
    !value.includes("\\") &&
    !/[\u0000-\u001f\u007f]/.test(value) &&
    !value.split("/").includes("..")
  );
}

const encodeUrlComponent = (value) =>
  encodeURIComponent(value).replace(
    /[!'()*]/g,
    (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
  );

const encodePath = (value) =>
  value.split("/").map(encodeUrlComponent).join("/");

const githubErrorStatus = (error) =>
  Number(error?.status ?? error?.response?.status);

function isRetryableGithubError(error) {
  const status = githubErrorStatus(error);
  if (status === 429 || (status >= 500 && status <= 504)) {
    return true;
  }
  if (status !== 403) {
    return false;
  }

  const headers = error?.response?.headers || {};
  const message = `${error?.message || ""} ${error?.response?.data?.message || ""}`;
  return (
    headers["retry-after"] !== undefined ||
    headers["x-ratelimit-remaining"] === "0" ||
    message.toLowerCase().includes("rate limit")
  );
}

function githubRetryDelayMs(error, attempt) {
  const headers = error?.response?.headers || {};
  const retryAfterSeconds = Number(headers["retry-after"]);
  if (Number.isFinite(retryAfterSeconds) && retryAfterSeconds >= 0) {
    return Math.min(Math.max(retryAfterSeconds * 1000, 1000), 120000);
  }

  const resetSeconds = Number(headers["x-ratelimit-reset"]);
  if (
    headers["x-ratelimit-remaining"] === "0" &&
    Number.isFinite(resetSeconds)
  ) {
    return Math.min(
      Math.max(resetSeconds * 1000 - Date.now() + 1000, 1000),
      120000,
    );
  }

  const status = githubErrorStatus(error);
  const baseDelay = status === 403 || status === 429 ? 60000 : 2000;
  return Math.min(baseDelay * 2 ** (attempt - 1), 120000);
}

async function withGithubRetry(
  operation,
  {
    core,
    sleep = (milliseconds) =>
      new Promise((resolve) => setTimeout(resolve, milliseconds)),
  },
) {
  for (let attempt = 1; attempt <= GITHUB_RETRY_ATTEMPTS; attempt += 1) {
    try {
      return await operation();
    } catch (error) {
      if (
        attempt === GITHUB_RETRY_ATTEMPTS ||
        !isRetryableGithubError(error)
      ) {
        throw error;
      }
      const delay = githubRetryDelayMs(error, attempt);
      core.warning(
        `GitHub API request failed with status ${githubErrorStatus(error)}; ` +
          `retrying in ${Math.ceil(delay / 1000)} seconds ` +
          `(attempt ${attempt + 1} of ${GITHUB_RETRY_ATTEMPTS}).`,
      );
      await sleep(delay);
    }
  }

  throw new Error("GitHub API retry loop ended unexpectedly.");
}

async function findReviewComment({ github, context, prNumber }) {
  const comments = await github.paginate(github.rest.issues.listComments, {
    owner: context.repo.owner,
    repo: context.repo.repo,
    issue_number: prNumber,
    per_page: 100,
  });
  return comments.find(
    (comment) =>
      comment.user?.login === "github-actions[bot]" &&
      comment.user?.type === "Bot" &&
      comment.body?.startsWith(`${MARKER}\n`),
  );
}

async function upsertReviewComment({ github, context, core, prNumber, body }) {
  const existing = await findReviewComment({ github, context, prNumber });
  if (existing) {
    await github.rest.issues.updateComment({
      owner: context.repo.owner,
      repo: context.repo.repo,
      comment_id: existing.id,
      body,
    });
    core.info(`Updated Codex security review comment #${existing.id}.`);
    return;
  }

  await github.rest.issues.createComment({
    owner: context.repo.owner,
    repo: context.repo.repo,
    issue_number: prNumber,
    body,
  });
  core.info(`Posted Codex security review on PR #${prNumber}.`);
}

async function getPullRequest({ github, context, prNumber }) {
  const { data: pullRequest } = await github.rest.pulls.get({
    owner: context.repo.owner,
    repo: context.repo.repo,
    pull_number: prNumber,
  });
  if (
    pullRequest.state !== "open" ||
    pullRequest.base.repo.full_name !==
      `${context.repo.owner}/${context.repo.repo}` ||
    pullRequest.base.ref !== "main"
  ) {
    return null;
  }
  return pullRequest;
}

async function getLiveMainSha({ github, context }) {
  const { data: mainRef } = await github.rest.git.getRef({
    owner: context.repo.owner,
    repo: context.repo.repo,
    ref: "heads/main",
  });
  const sha = mainRef.object?.sha || "";
  if (mainRef.object?.type !== "commit" || !/^[0-9a-f]{40,64}$/.test(sha)) {
    throw new Error("refs/heads/main did not resolve to a commit SHA.");
  }
  return sha;
}

async function ensureCurrentReviewLabel({ github, context }) {
  try {
    await github.rest.issues.getLabel({
      owner: context.repo.owner,
      repo: context.repo.repo,
      name: CURRENT_REVIEW_LABEL,
    });
    return;
  } catch (error) {
    if (error?.status !== 404) {
      throw error;
    }
  }

  try {
    await github.rest.issues.createLabel({
      owner: context.repo.owner,
      repo: context.repo.repo,
      name: CURRENT_REVIEW_LABEL,
      color: "1d76db",
      description: "The posted Codex security review matches its recorded range.",
    });
  } catch (error) {
    // Another posting job may create the repository label concurrently.
    if (error?.status !== 422) {
      throw error;
    }
  }
}

async function markReviewCurrent({ github, context, prNumber }) {
  await ensureCurrentReviewLabel({ github, context });
  await github.rest.issues.addLabels({
    owner: context.repo.owner,
    repo: context.repo.repo,
    issue_number: prNumber,
    labels: [CURRENT_REVIEW_LABEL],
  });
}

async function clearCurrentReview({ github, context, prNumber }) {
  try {
    await github.rest.issues.removeLabel({
      owner: context.repo.owner,
      repo: context.repo.repo,
      issue_number: prNumber,
      name: CURRENT_REVIEW_LABEL,
    });
  } catch (error) {
    if (error?.status !== 404) {
      throw error;
    }
  }
}

async function reviewRangeIsCurrent({
  github,
  context,
  prNumber,
  baseSha,
  headSha,
  headRepo,
}) {
  const [pullRequest, liveMainSha] = await Promise.all([
    getPullRequest({ github, context, prNumber }),
    getLiveMainSha({ github, context }),
  ]);
  return (
    pullRequest !== null &&
    liveMainSha === baseSha &&
    pullRequest.head.sha === headSha &&
    pullRequest.head.repo?.full_name === headRepo
  );
}

async function prepare({ github, context, core }) {
  let prNumber;
  let requestedHeadSha;
  if (context.eventName === "pull_request_target") {
    prNumber = Number(context.payload.pull_request?.number);
    requestedHeadSha = context.payload.pull_request?.head?.sha || "";
  } else if (context.eventName === "issue_comment") {
    prNumber = Number(context.payload.issue?.number);
    const command = context.payload.comment?.body || "";
    const match = /^@buzz-security-review ([0-9a-f]{40})$/.exec(command);
    if (!match) {
      core.setFailed(
        `Review commands must be exactly "${REVIEW_COMMAND} <full-head-sha>".`,
      );
      return;
    }
    requestedHeadSha = match[1];
  } else {
    core.setFailed(`Unsupported review trigger: ${context.eventName}.`);
    return;
  }

  if (!Number.isSafeInteger(prNumber) || prNumber <= 0) {
    core.setFailed("Invalid pull request number for security review.");
    return;
  }

  const pullRequest = await getPullRequest({ github, context, prNumber });
  if (!pullRequest) {
    core.setFailed(
      `Pull request #${prNumber} is not an open PR targeting main.`,
    );
    return;
  }
  if (!pullRequest.head.repo?.full_name) {
    core.setFailed(
      `Pull request #${prNumber} has no available head repository.`,
    );
    return;
  }
  if (
    context.eventName === "pull_request_target" &&
    !isOrganizationMember(pullRequest.author_association)
  ) {
    core.info(
      `Pull request #${prNumber} requires authorization from a Block organization member.`,
    );
    return;
  }
  if (pullRequest.head.sha !== requestedHeadSha) {
    core.setFailed(
      `Pull request #${prNumber} moved after this review was authorized. ` +
        `Use "${reviewCommand(pullRequest.head.sha)}" to review the current head.`,
    );
    return;
  }

  const baseSha = await getLiveMainSha({ github, context });
  const commitRange = `${baseSha}...${pullRequest.head.sha}`;
  core.setOutput("authorized", "true");
  core.setOutput("pr_number", String(prNumber));
  core.setOutput("trigger_actor", context.actor);
  core.setOutput("base_sha", baseSha);
  core.setOutput("head_sha", pullRequest.head.sha);
  core.setOutput("head_repo", pullRequest.head.repo.full_name);
  core.setOutput("commit_range", commitRange);
}

function setReconciliationOutputs(
  core,
  {
    prNumbers,
    mainSha,
    shouldContinue = false,
    nextAfter = 0,
    nextPass = 1,
  },
) {
  core.setOutput(
    "pr_numbers",
    JSON.stringify(prNumbers.length > 0 ? prNumbers : [0]),
  );
  core.setOutput("main_sha", mainSha);
  core.setOutput("should_continue", String(shouldContinue));
  core.setOutput("next_after", String(nextAfter));
  core.setOutput("next_pass", String(nextPass));
}

async function prepareBaseReconciliation({ github, context, core }) {
  const reconciliation = context.payload.client_payload || {};
  const afterPrNumber = Number(reconciliation.after_pr || 0);
  if (!Number.isSafeInteger(afterPrNumber) || afterPrNumber < 0) {
    throw new Error("Invalid reconciliation cursor.");
  }
  const pass = Number(reconciliation.pass || 1);
  if (
    !Number.isSafeInteger(pass) ||
    pass < 1 ||
    pass > MAX_RECONCILIATION_PASSES
  ) {
    throw new Error("Invalid reconciliation pass.");
  }

  const requestedMainSha =
    context.eventName === "push"
      ? context.sha
      : reconciliation.main_sha || "";
  if (requestedMainSha && !/^[0-9a-f]{40,64}$/.test(requestedMainSha)) {
    throw new Error("Invalid reconciliation main SHA.");
  }
  const liveMainSha = await getLiveMainSha({ github, context });
  const mainSha = requestedMainSha || liveMainSha;
  if (mainSha !== liveMainSha) {
    core.info(
      `Skipping reconciliation for superseded main commit ${mainSha}.`,
    );
    setReconciliationOutputs(core, { prNumbers: [], mainSha, nextPass: pass });
    return;
  }

  const issues = await github.paginate(github.rest.issues.listForRepo, {
    owner: context.repo.owner,
    repo: context.repo.repo,
    state: "open",
    labels: CURRENT_REVIEW_LABEL,
    per_page: 100,
  });
  const prNumbers = [
    ...new Set(
      issues
        .filter((issue) => issue.pull_request)
        .map((issue) => issue.number)
        .filter((prNumber) => Number.isSafeInteger(prNumber) && prNumber > 0),
    ),
  ]
    .sort((left, right) => left - right)
    .filter((prNumber) => prNumber > afterPrNumber);
  const batch = prNumbers.slice(0, RECONCILIATION_BATCH_SIZE);
  const hasMore = prNumbers.length > RECONCILIATION_BATCH_SIZE;
  const startRetryPass =
    (batch.length > 0 || afterPrNumber > 0) &&
    !hasMore &&
    pass < MAX_RECONCILIATION_PASSES;
  setReconciliationOutputs(core, {
    prNumbers: batch,
    mainSha,
    shouldContinue: hasMore || startRetryPass,
    nextAfter: hasMore ? batch.at(-1) : 0,
    nextPass: startRetryPass ? pass + 1 : pass,
  });
}

async function invalidatePullRequestUpdate({ github, context, core }) {
  await invalidate({
    github,
    context,
    core,
    existingOnlyForOrganizationMembers: true,
  });
}

async function invalidate({
  github,
  context,
  core,
  prNumber: requestedPrNumber,
  existingOnly = false,
  existingOnlyForOrganizationMembers = false,
}) {
  const prNumber = Number(
    requestedPrNumber ?? context.payload.pull_request?.number,
  );
  if (!Number.isSafeInteger(prNumber) || prNumber <= 0) {
    throw new Error("Invalid pull request number for review invalidation.");
  }

  const pullRequest = await getPullRequest({ github, context, prNumber });
  if (!pullRequest) {
    await clearCurrentReview({ github, context, prNumber });
    core.notice(`Skipping review invalidation for ineligible PR #${prNumber}.`);
    return;
  }

  const existing = await findReviewComment({ github, context, prNumber });
  const shouldOnlyUpdateExisting =
    existingOnly ||
    (existingOnlyForOrganizationMembers &&
      isOrganizationMember(pullRequest.author_association));
  if (!existing && shouldOnlyUpdateExisting) {
    if (existingOnly || hasCurrentReviewLabel(pullRequest)) {
      await clearCurrentReview({ github, context, prNumber });
    }
    core.info(`PR #${prNumber} has no Codex security review to invalidate.`);
    return;
  }

  const liveMainSha = await getLiveMainSha({ github, context });
  const currentPrefix =
    `${MARKER}\n` +
    `${completedMarker(liveMainSha, pullRequest.head.sha)}\n`;
  if (existing?.body?.startsWith(currentPrefix)) {
    core.info(`PR #${prNumber} already has a review for the current range.`);
    return;
  }

  const body = `${MARKER}
${STALE_MARKER}
## 🔐 Codex Security Review

> **Status: review required for the current range.**
>
> The current range is \`${liveMainSha}...${pullRequest.head.sha}\`.
> A new review must complete for this exact range. When manual authorization
> is required, a Block organization member must comment exactly
> \`${reviewCommand(pullRequest.head.sha)}\` to authorize a new review.
> Any previous review applies only to its recorded range.
`;

  if (existing?.body === body) {
    core.info(`PR #${prNumber} already has the current stale-review notice.`);
    await clearCurrentReview({ github, context, prNumber });
    return;
  }

  await upsertReviewComment({ github, context, core, prNumber, body });
  await clearCurrentReview({ github, context, prNumber });
}

async function post({ github, context, core }) {
  const rawReview = process.env.REVIEW_JSON || "";
  if (rawReview.length === 0 || rawReview.length > 120000) {
    throw new Error("Codex output is empty or exceeds the renderer limit.");
  }

  const review = JSON.parse(rawReview);
  requireKeys(review, ["overall_risk", "summary", "findings", "notes"], "review");
  if (!RISKS.includes(review.overall_risk)) {
    throw new Error("Review has an invalid overall risk.");
  }
  if (!Array.isArray(review.findings) || review.findings.length > 10) {
    throw new Error("Review findings must be an array with at most 10 entries.");
  }
  if (!Array.isArray(review.notes) || review.notes.length > 5) {
    throw new Error("Review notes must be an array with at most 5 entries.");
  }

  const prNumber = Number(process.env.REVIEW_PR_NUMBER);
  if (!Number.isSafeInteger(prNumber) || prNumber <= 0) {
    throw new Error("Invalid reviewed pull request number.");
  }
  const baseSha = process.env.REVIEW_BASE_SHA || "";
  const headSha = process.env.REVIEW_HEAD_SHA || "";
  const headRepo = process.env.REVIEW_HEAD_REPO || "";
  const commitRange = process.env.REVIEW_COMMIT_RANGE || "";
  if (!/^[0-9a-f]{40,64}$/.test(baseSha) || !/^[0-9a-f]{40,64}$/.test(headSha)) {
    throw new Error("Invalid reviewed commit SHA.");
  }
  if (commitRange !== `${baseSha}...${headSha}`) {
    throw new Error("Invalid reviewed commit range.");
  }
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(headRepo)) {
    throw new Error("Invalid reviewed head repository.");
  }

  const existingReview = {
    github,
    context,
    core,
    prNumber,
    existingOnly: true,
  };
  const reviewedRange = {
    github,
    context,
    prNumber,
    baseSha,
    headSha,
    headRepo,
  };

  const [pullRequest, liveMainSha] = await Promise.all([
    getPullRequest({ github, context, prNumber }),
    getLiveMainSha({ github, context }),
  ]);
  if (
    !pullRequest ||
    liveMainSha !== baseSha ||
    pullRequest.head.sha !== headSha ||
    pullRequest.head.repo?.full_name !== headRepo
  ) {
    core.notice(`Skipping stale review for ${commitRange} on PR #${prNumber}.`);
    await invalidate(existingReview);
    return;
  }

  const files = await github.paginate(github.rest.pulls.listFiles, {
    owner: context.repo.owner,
    repo: context.repo.repo,
    pull_number: prNumber,
    per_page: 100,
  });
  if (files.length !== pullRequest.changed_files) {
    throw new Error(
      `Expected ${pullRequest.changed_files} changed files, but GitHub returned ${files.length}.`,
    );
  }
  const changedFiles = new Set(files.map((file) => file.filename));

  const findingKeys = [
    "severity",
    "category",
    "title",
    "path",
    "line",
    "description",
    "impact",
    "recommendation",
  ];
  const [headOwner, headName] = headRepo.split("/");
  const renderedFindings = review.findings.map((finding, index) => {
    const label = `finding ${index + 1}`;
    requireKeys(finding, findingKeys, label);
    if (!SEVERITIES.has(finding.severity)) {
      throw new Error(`${label} has an invalid severity.`);
    }
    if (!CATEGORIES.has(finding.category)) {
      throw new Error(`${label} has an invalid category.`);
    }
    if (!validPath(finding.path) || !changedFiles.has(finding.path)) {
      throw new Error(`${label} does not reference a changed file.`);
    }
    if (
      !Number.isSafeInteger(finding.line) ||
      finding.line < 1 ||
      finding.line > 10000000
    ) {
      throw new Error(`${label} has an invalid line number.`);
    }

    const location =
      `https://github.com/${encodeUrlComponent(headOwner)}/${encodeUrlComponent(headName)}` +
      `/blob/${headSha}/${encodePath(finding.path)}#L${finding.line}`;
    const pathLabel = safeCodeText(
      `${finding.path}:${finding.line}`,
      520,
      `${label} location`,
    );
    return [
      `#### [${finding.severity}] ${safeCodeText(finding.title, 200, `${label} title`)}`,
      `- **Category**: ${finding.category}`,
      `- **Location**: ${pathLabel} ([source](${location}))`,
      `- **Description**: ${safeCodeText(finding.description, 1500, `${label} description`)}`,
      `- **Impact**: ${safeCodeText(finding.impact, 1500, `${label} impact`)}`,
      `- **Recommendation**: ${safeCodeText(finding.recommendation, 1500, `${label} recommendation`)}`,
    ].join("\n");
  });

  const highestFindingRisk = review.findings.reduce(
    (highest, finding) => Math.max(highest, RISKS.indexOf(finding.severity)),
    0,
  );
  const overallRisk = RISKS[
    Math.max(RISKS.indexOf(review.overall_risk), highestFindingRisk)
  ];
  const findingsMarkdown = renderedFindings.length
    ? renderedFindings.join("\n\n")
    : "No concrete security, correctness, or reliability findings were identified.";
  const notesMarkdown = review.notes.length
    ? review.notes
        .map((note, index) => `- ${safeCodeText(note, 1000, `note ${index + 1}`)}`)
        .join("\n")
    : "- No additional limitations were reported.";

  const triggerActor = process.env.REVIEW_TRIGGER_ACTOR || "";
  if (!/^[A-Za-z0-9-]{1,39}$/.test(triggerActor)) {
    throw new Error("Invalid review trigger actor.");
  }
  const model = process.env.CODEX_MODEL || "";
  if (!/^[A-Za-z0-9._-]{1,100}$/.test(model)) {
    throw new Error("Invalid review model name.");
  }
  const workflowRun =
    `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}` +
    `/actions/runs/${process.env.GITHUB_RUN_ID}`;
  const body = `${MARKER}
${completedMarker(baseSha, headSha)}
## 🔐 Codex Security Review

> **Note**: This is an automated, security-focused review generated by Codex.
> Use it as a supplement to human review; false positives are possible.
>
> **Scope**
> - Exact PR diff: \`${commitRange}\`
> - Model: ${model}
>
> 💡 *Click "edited" above to see earlier reviews for this PR.*

---

## Review Summary

**Overall Risk**: ${overallRisk}

${safeCodeText(review.summary, 2000, "review summary")}

### Findings

${findingsMarkdown}

### Notes

${notesMarkdown}

---

<sub>Generated by [Codex Security Review](https://github.com/openai/codex-action) |
Requested by: \`@${triggerActor}\` |
[Workflow run](${workflowRun})</sub>`;

  if (body.length > 60000) {
    throw new Error("Rendered security review exceeds the GitHub comment limit.");
  }

  // Register the pending write before the final freshness check. If main moves
  // now, either this check clears the label or the main-push reconciler sees it.
  await markReviewCurrent({ github, context, prNumber });
  if (!(await reviewRangeIsCurrent(reviewedRange))) {
    core.notice(
      `Skipping review because ${commitRange} moved while PR #${prNumber} was rendering.`,
    );
    await invalidate(existingReview);
    return;
  }

  await upsertReviewComment({ github, context, core, prNumber, body });

  if (!(await reviewRangeIsCurrent(reviewedRange))) {
    core.notice(
      `Review range ${commitRange} moved while posting on PR #${prNumber}; marking it stale.`,
    );
    await invalidate(existingReview);
  }
}

module.exports = {
  invalidate,
  invalidatePullRequestUpdate,
  post,
  prepare,
  prepareBaseReconciliation,
  withGithubRetry,
};
