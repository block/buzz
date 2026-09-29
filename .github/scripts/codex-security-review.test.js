"use strict";

const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const {
  invalidate,
  invalidatePullRequestUpdate,
  post,
  prepare,
  prepareBaseReconciliation,
  withGithubRetry,
} = require("./codex-security-review.js");

const BASE_SHA = "a".repeat(40);
const OLD_BASE_SHA = "b".repeat(40);
const HEAD_SHA = "c".repeat(40);
const OTHER_HEAD_SHA = "d".repeat(40);
const NEW_BASE_SHA = "e".repeat(40);
const MARKER = "<!-- codex-security-review -->";
const CURRENT_REVIEW_LABEL = "codex-security-review-current";

function pullRequest({
  authorAssociation = "CONTRIBUTOR",
  baseSha = OLD_BASE_SHA,
  headSha = HEAD_SHA,
  labels = [],
} = {}) {
  return {
    author_association: authorAssociation,
    state: "open",
    base: {
      ref: "main",
      sha: baseSha,
      repo: { full_name: "block/buzz" },
    },
    head: {
      sha: headSha,
      repo: { full_name: "outside/buzz" },
    },
    changed_files: 1,
    labels,
  };
}

function harness({
  pull = pullRequest(),
  comments = [],
  files = [],
  labeledIssues = [],
  liveMainShas = [BASE_SHA],
} = {}) {
  const storedComments = [...comments];
  const created = [];
  const updated = [];
  const addedLabels = [];
  const removedLabels = [];
  const removeLabelCalls = [];
  const outputs = new Map();
  const failures = [];
  const notices = [];
  const info = [];
  const warnings = [];
  const listComments = async () => storedComments;
  const listFiles = async () => files;
  let labelExists = false;
  let mainRefRead = 0;
  const github = {
    paginate: async (method, args) => method(args),
    rest: {
      git: {
        getRef: async () => {
          const sha =
            liveMainShas[Math.min(mainRefRead, liveMainShas.length - 1)];
          mainRefRead += 1;
          return { data: { object: { type: "commit", sha } } };
        },
      },
      issues: {
        listComments,
        listForRepo: async () => labeledIssues,
        getLabel: async () => {
          if (!labelExists) {
            throw Object.assign(new Error("not found"), { status: 404 });
          }
          return { data: { name: CURRENT_REVIEW_LABEL } };
        },
        createLabel: async () => {
          labelExists = true;
        },
        addLabels: async (input) => {
          labelExists = true;
          addedLabels.push(input);
        },
        removeLabel: async (input) => {
          removeLabelCalls.push(input);
          if (!labelExists) {
            throw Object.assign(new Error("not found"), { status: 404 });
          }
          labelExists = false;
          removedLabels.push(input);
        },
        createComment: async (input) => {
          created.push(input);
          storedComments.push(
            reviewComment(input.body, { id: 100 + storedComments.length }),
          );
        },
        updateComment: async (input) => {
          updated.push(input);
          const comment = storedComments.find(
            (candidate) => candidate.id === input.comment_id,
          );
          if (comment) {
            comment.body = input.body;
          }
        },
      },
      pulls: {
        get: async () => ({ data: pull }),
        listFiles,
      },
    },
  };
  const core = {
    info: (message) => info.push(message),
    notice: (message) => notices.push(message),
    warning: (message) => warnings.push(message),
    setFailed: (message) => failures.push(message),
    setOutput: (name, value) => outputs.set(name, value),
  };
  const context = {
    actor: "block-member",
    eventName: "issue_comment",
    payload: {
      comment: { body: `@buzz-security-review ${HEAD_SHA}` },
      issue: { number: 6816 },
    },
    repo: { owner: "block", repo: "buzz" },
  };
  return {
    context,
    core,
    addedLabels,
    created,
    failures,
    github,
    info,
    notices,
    outputs,
    removeLabelCalls,
    removedLabels,
    storedComments,
    updated,
    warnings,
  };
}

function reviewComment(body, { id = 42 } = {}) {
  return {
    id,
    body,
    user: { login: "github-actions[bot]", type: "Bot" },
  };
}

const NO_FINDINGS_REVIEW = {
  overall_risk: "NONE",
  summary: "No findings.",
  findings: [],
  notes: [],
};

async function postReview(state, review = NO_FINDINGS_REVIEW) {
  const environment = {
    CODEX_MODEL: "gpt-5.6-sol",
    GITHUB_REPOSITORY: "block/buzz",
    GITHUB_RUN_ID: "1234",
    GITHUB_SERVER_URL: "https://github.com",
    REVIEW_BASE_SHA: BASE_SHA,
    REVIEW_COMMIT_RANGE: `${BASE_SHA}...${HEAD_SHA}`,
    REVIEW_HEAD_REPO: "outside/buzz",
    REVIEW_HEAD_SHA: HEAD_SHA,
    REVIEW_JSON: JSON.stringify(review),
    REVIEW_PR_NUMBER: "6816",
    REVIEW_TRIGGER_ACTOR: "block-member",
  };
  const previous = Object.fromEntries(
    Object.keys(environment).map((key) => [key, process.env[key]]),
  );
  Object.assign(process.env, environment);

  try {
    await post(state);
  } finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  }
}

test("prepare binds a member command to the named head SHA", async () => {
  const current = harness();
  await prepare(current);

  assert.deepEqual(current.failures, []);
  assert.equal(current.outputs.get("authorized"), "true");
  assert.equal(current.outputs.get("head_sha"), HEAD_SHA);
  assert.equal(current.outputs.get("base_sha"), BASE_SHA);
  assert.notEqual(current.outputs.get("base_sha"), OLD_BASE_SHA);
  assert.equal(
    current.outputs.get("commit_range"),
    `${BASE_SHA}...${HEAD_SHA}`,
  );

  const moved = harness({ pull: pullRequest({ headSha: OTHER_HEAD_SHA }) });
  await prepare(moved);

  assert.equal(moved.outputs.size, 0);
  assert.equal(moved.failures.length, 1);
  assert.match(moved.failures[0], new RegExp(OTHER_HEAD_SHA));
  assert.match(
    moved.failures[0],
    new RegExp(`@buzz-security-review ${OTHER_HEAD_SHA}`),
  );
});

test("pull request authorization uses the live author association", async () => {
  const member = harness({
    pull: pullRequest({ authorAssociation: "MEMBER" }),
  });
  member.context.eventName = "pull_request_target";
  member.context.payload.pull_request = {
    number: 6816,
    head: { sha: HEAD_SHA },
    author_association: "CONTRIBUTOR",
  };

  await prepare(member);

  assert.equal(member.outputs.get("authorized"), "true");
  assert.deepEqual(member.failures, []);

  const external = harness({
    pull: pullRequest({ authorAssociation: "CONTRIBUTOR" }),
  });
  external.context.eventName = "pull_request_target";
  external.context.payload.pull_request = {
    number: 6816,
    head: { sha: HEAD_SHA },
    author_association: "MEMBER",
  };

  await prepare(external);

  assert.equal(external.outputs.get("authorized"), undefined);
  assert.deepEqual(external.failures, []);
  assert.match(external.info.at(-1), /requires authorization/);
});

test("PR mutation jobs use pull request write permission", () => {
  const workflow = readFileSync(
    path.join(__dirname, "../workflows/codex-security-review.yml"),
    "utf8",
  );
  for (const jobName of [
    "reconcile-base-reviews",
    "invalidate-previous-review",
    "post-review",
  ]) {
    const start = workflow.indexOf(`  ${jobName}:\n`);
    assert.notEqual(start, -1, `missing workflow job ${jobName}`);
    const remainder = workflow.slice(start + 2);
    const nextJob = remainder.search(/^  [a-z][a-z0-9-]*:\n/m);
    const job =
      nextJob === -1
        ? workflow.slice(start)
        : workflow.slice(start, start + 2 + nextJob);
    assert.match(job, /^      pull-requests: write$/m);
    assert.doesNotMatch(job, /^      issues: write$/m);
  }
});

test("prepare rejects review commands without a full exact SHA", async () => {
  const state = harness();
  state.context.payload.comment.body = "@buzz-security-review";

  await prepare(state);

  assert.equal(state.outputs.size, 0);
  assert.equal(state.failures.length, 1);
  assert.match(state.failures[0], /<full-head-sha>/);
});

test("invalidation compares the complete base and head range", async () => {
  const stale = harness({
    comments: [
      reviewComment(
        `${MARKER}\n<!-- codex-security-review-range:${OLD_BASE_SHA}...${HEAD_SHA} -->\nold review`,
      ),
    ],
  });
  await stale.github.rest.issues.addLabels({
    issue_number: 6816,
    labels: [CURRENT_REVIEW_LABEL],
  });

  await invalidate({
    github: stale.github,
    context: stale.context,
    core: stale.core,
    prNumber: 6816,
    existingOnly: true,
  });

  assert.equal(stale.updated.length, 1);
  assert.match(stale.updated[0].body, /review required for the current range/);
  assert.ok(stale.updated[0].body.includes(`${BASE_SHA}...${HEAD_SHA}`));
  assert.match(
    stale.updated[0].body,
    new RegExp(`@buzz-security-review ${HEAD_SHA}`),
  );
  assert.equal(stale.removedLabels.length, 1);

  await invalidate({
    github: stale.github,
    context: stale.context,
    core: stale.core,
    prNumber: 6816,
    existingOnly: true,
  });

  assert.equal(stale.updated.length, 1);
  assert.match(stale.info.at(-1), /already has the current stale-review notice/);

  const current = harness({
    comments: [
      reviewComment(
        `${MARKER}\n<!-- codex-security-review-range:${BASE_SHA}...${HEAD_SHA} -->\ncurrent review`,
      ),
    ],
  });
  await invalidate({
    github: current.github,
    context: current.context,
    core: current.core,
    prNumber: 6816,
    existingOnly: true,
  });

  assert.equal(current.updated.length, 0);
  assert.match(current.info.at(-1), /current range/);
});

test("base reconciliation batches every labeled review with a durable cursor", async () => {
  const labeledIssues = Array.from({ length: 34 }, (_, index) => ({
    number: index + 1,
    pull_request: {},
  }));
  const first = harness({ labeledIssues });
  first.context.eventName = "repository_dispatch";
  first.context.payload.client_payload = {
    after_pr: "1",
    main_sha: BASE_SHA,
  };

  await prepareBaseReconciliation(first);

  const firstBatch = JSON.parse(first.outputs.get("pr_numbers"));
  assert.equal(firstBatch.length, 32);
  assert.equal(firstBatch[0], 2);
  assert.equal(firstBatch.at(-1), 33);
  assert.equal(first.outputs.get("main_sha"), BASE_SHA);
  assert.equal(first.outputs.get("should_continue"), "true");
  assert.equal(first.outputs.get("next_after"), "33");
  assert.equal(first.outputs.get("next_pass"), "1");

  const second = harness({ labeledIssues });
  second.context.eventName = "repository_dispatch";
  second.context.payload.client_payload = {
    after_pr: "33",
    main_sha: BASE_SHA,
  };

  await prepareBaseReconciliation(second);

  assert.deepEqual(JSON.parse(second.outputs.get("pr_numbers")), [34]);
  assert.equal(second.outputs.get("should_continue"), "true");
  assert.equal(second.outputs.get("next_after"), "0");
  assert.equal(second.outputs.get("next_pass"), "2");

  const disappearedTail = harness();
  disappearedTail.context.eventName = "repository_dispatch";
  disappearedTail.context.payload.client_payload = {
    after_pr: "33",
    main_sha: BASE_SHA,
    pass: "1",
  };

  await prepareBaseReconciliation(disappearedTail);

  assert.deepEqual(
    JSON.parse(disappearedTail.outputs.get("pr_numbers")),
    [0],
  );
  assert.equal(disappearedTail.outputs.get("should_continue"), "true");
  assert.equal(disappearedTail.outputs.get("next_after"), "0");
  assert.equal(disappearedTail.outputs.get("next_pass"), "2");

  const retry = harness({ labeledIssues: [labeledIssues.at(-1)] });
  retry.context.eventName = "repository_dispatch";
  retry.context.payload.client_payload = {
    after_pr: "0",
    main_sha: BASE_SHA,
    pass: "2",
  };

  await prepareBaseReconciliation(retry);

  assert.deepEqual(JSON.parse(retry.outputs.get("pr_numbers")), [34]);
  assert.equal(retry.outputs.get("should_continue"), "false");
  assert.equal(retry.outputs.get("next_after"), "0");
  assert.equal(retry.outputs.get("next_pass"), "2");
});

test("base reconciliation stops a continuation from an older main commit", async () => {
  const state = harness({
    labeledIssues: [{ number: 6816, pull_request: {} }],
    liveMainShas: [NEW_BASE_SHA],
  });
  state.context.eventName = "repository_dispatch";
  state.context.payload.client_payload = {
    after_pr: "256",
    main_sha: BASE_SHA,
  };

  await prepareBaseReconciliation(state);

  assert.deepEqual(JSON.parse(state.outputs.get("pr_numbers")), [0]);
  assert.equal(state.outputs.get("main_sha"), BASE_SHA);
  assert.equal(state.outputs.get("should_continue"), "false");
  assert.equal(state.outputs.get("next_after"), "0");
  assert.equal(state.outputs.get("next_pass"), "1");
  assert.match(state.info.at(-1), /superseded main commit/);
});

test("GitHub rate limits use bounded retry delays", async () => {
  const waits = [];
  const warnings = [];
  let attempts = 0;
  const rateLimitError = Object.assign(new Error("secondary rate limit"), {
    status: 403,
    response: {
      status: 403,
      headers: { "retry-after": "0" },
      data: { message: "secondary rate limit" },
    },
  });

  const result = await withGithubRetry(
    async () => {
      attempts += 1;
      if (attempts < 3) {
        throw rateLimitError;
      }
      return "completed";
    },
    {
      core: { warning: (message) => warnings.push(message) },
      sleep: async (milliseconds) => waits.push(milliseconds),
    },
  );

  assert.equal(result, "completed");
  assert.equal(attempts, 3);
  assert.deepEqual(waits, [1000, 1000]);
  assert.equal(warnings.length, 2);
});

test("base reconciliation does not create comments on unreviewed PRs", async () => {
  const state = harness();

  await invalidate({
    github: state.github,
    context: state.context,
    core: state.core,
    prNumber: 6816,
    existingOnly: true,
  });

  assert.equal(state.created.length, 0);
  assert.equal(state.updated.length, 0);
});

test("pull request updates invalidate member reviews without adding placeholders", async () => {
  const reviewed = harness({
    pull: pullRequest({
      authorAssociation: "MEMBER",
      headSha: OTHER_HEAD_SHA,
    }),
    comments: [
      reviewComment(
        `${MARKER}\n<!-- codex-security-review-range:${BASE_SHA}...${HEAD_SHA} -->\nold review`,
      ),
    ],
  });
  reviewed.context.eventName = "pull_request_target";
  reviewed.context.payload.pull_request = {
    number: 6816,
    author_association: "CONTRIBUTOR",
  };
  await reviewed.github.rest.issues.addLabels({
    issue_number: 6816,
    labels: [CURRENT_REVIEW_LABEL],
  });

  await invalidatePullRequestUpdate(reviewed);

  assert.equal(reviewed.updated.length, 1);
  assert.ok(
    reviewed.updated[0].body.includes(`${BASE_SHA}...${OTHER_HEAD_SHA}`),
  );
  assert.equal(reviewed.removedLabels.length, 1);

  const unreviewed = harness({
    pull: pullRequest({ authorAssociation: "OWNER" }),
  });
  unreviewed.context.eventName = "pull_request_target";
  unreviewed.context.payload.pull_request = {
    number: 6816,
    author_association: "CONTRIBUTOR",
  };

  await invalidatePullRequestUpdate(unreviewed);

  assert.equal(unreviewed.created.length, 0);
  assert.equal(unreviewed.updated.length, 0);
  assert.equal(unreviewed.removeLabelCalls.length, 0);

  const external = harness({
    pull: pullRequest({ authorAssociation: "CONTRIBUTOR" }),
  });
  external.context.eventName = "pull_request_target";
  external.context.payload.pull_request = {
    number: 6816,
    author_association: "MEMBER",
  };

  await invalidatePullRequestUpdate(external);

  assert.equal(external.created.length, 1);
  assert.match(external.created[0].body, /review required for the current range/);
});

test("post preserves finding text while rendering it as inert code", async () => {
  const findingPath = "src/x)www.example.com/review.js";
  const state = harness({ files: [{ filename: findingPath }] });
  const summary =
    "Keep <script>, a && b, @security, https://example.com, a \\ path, and ＆ distinct.";
  const title = "Do not rewrite `a && b`";
  const description =
    "Compare <script> with @ops and https://example.com before changing it.";
  const impact = "A reader cannot copy `a && b` when punctuation changes.";
  const recommendation = "Preserve ``nested backticks`` and visible operators.";
  const note = "Remove bidi controls only: a\u061cb\u200ec\u200fd\u202ee.";
  const review = {
    overall_risk: "MEDIUM",
    summary,
    findings: [
      {
        severity: "MEDIUM",
        category: "Injection",
        title,
        path: findingPath,
        line: 17,
        description,
        impact,
        recommendation,
      },
    ],
    notes: [note],
  };
  await postReview(state, review);

  assert.equal(state.created.length, 1);
  const body = state.created[0].body;
  for (const expected of [
    summary,
    title,
    description,
    impact,
    recommendation,
  ]) {
    assert.ok(body.includes(expected), `missing exact text: ${expected}`);
  }
  assert.ok(body.includes("` " + summary + " `"));
  assert.ok(body.includes("`` " + title + " ``"));
  assert.ok(body.includes("``` " + recommendation + " ```"));
  assert.ok(body.includes("a b c d e."));
  assert.ok(body.includes("src/x%29www.example.com/review.js#L17"));
  for (const bidiControl of ["\u061c", "\u200e", "\u200f", "\u202e"]) {
    assert.ok(!body.includes(bidiControl));
  }
  assert.ok(!body.includes("＠"));
  assert.ok(!body.includes("https："));
  assert.match(
    body,
    new RegExp(`codex-security-review-range:${BASE_SHA}\\.\\.\\.${HEAD_SHA}`),
  );
  assert.equal(state.addedLabels.length, 1);
  assert.equal(state.removedLabels.length, 0);
});

test("post registers itself before its pre-write freshness check", async () => {
  const state = harness({
    files: [{ filename: "src/review.js" }],
    liveMainShas: [BASE_SHA, NEW_BASE_SHA],
  });
  await postReview(state);

  assert.equal(state.created.length, 0);
  assert.equal(state.addedLabels.length, 1);
  assert.equal(state.removedLabels.length, 1);
  assert.match(state.notices.at(-1), /moved while PR/);
});

test("post marks its comment stale when main moves during the write", async () => {
  const state = harness({
    files: [{ filename: "src/review.js" }],
    liveMainShas: [BASE_SHA, BASE_SHA, NEW_BASE_SHA, NEW_BASE_SHA],
  });
  await postReview(state);

  assert.equal(state.created.length, 1);
  assert.equal(state.updated.length, 1);
  assert.match(state.updated[0].body, /review required for the current range/);
  assert.ok(state.updated[0].body.includes(`${NEW_BASE_SHA}...${HEAD_SHA}`));
  assert.equal(state.removedLabels.length, 1);
});
