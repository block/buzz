import assert from "node:assert/strict";
import test from "node:test";

import {
  collectBranchSourcesFromAgentMessages,
  collectChatWorkBranchSources,
  deriveBranchFromAgentMessages,
  deriveBranchSourceFromAgentMessages,
  deriveChatWorkBranch,
  deriveChatWorkBranchSource,
  parseBranchFromCommand,
} from "./chatWorkBranch.ts";

function toolItem(command, overrides = {}) {
  return {
    id: overrides.id ?? command,
    type: "tool",
    renderClass: "generic",
    descriptor: { category: "shell" },
    title: "Ran command",
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: { command },
    result: "",
    isError: false,
    timestamp: overrides.timestamp ?? "2026-07-04T00:00:00Z",
    startedAt: overrides.timestamp ?? "2026-07-04T00:00:00Z",
    completedAt: null,
    channelId: overrides.channelId ?? "chat-1",
    turnId: overrides.turnId ?? "turn-1",
  };
}

test("worktree add with -b names the new branch", () => {
  assert.equal(
    parseBranchFromCommand("git worktree add ../wt/feature -b feature-x"),
    "feature-x",
  );
});

test("worktree add with existing branch uses the second positional", () => {
  assert.equal(
    parseBranchFromCommand("git worktree add ../wt/fix fix-panel"),
    "fix-panel",
  );
});

test("worktree add with only a path uses the basename", () => {
  assert.equal(
    parseBranchFromCommand("git worktree add /tmp/worktrees/chat-panel"),
    "chat-panel",
  );
});

test("checkout -b and switch -c name the branch", () => {
  assert.equal(
    parseBranchFromCommand("git checkout -b kenny/new-panel"),
    "kenny/new-panel",
  );
  assert.equal(parseBranchFromCommand("git switch -c wip"), "wip");
});

test("plain switch to an existing branch counts", () => {
  assert.equal(parseBranchFromCommand("git switch main"), "main");
});

test("compound commands take the last branch operation", () => {
  assert.equal(
    parseBranchFromCommand(
      "cd /repo && git fetch origin && git worktree add ../wt -b first && git -C ../wt checkout -b second",
    ),
    "second",
  );
});

test("file checkouts, detached heads, and non-git commands are ignored", () => {
  assert.equal(parseBranchFromCommand("git checkout -- src/app.ts"), null);
  assert.equal(parseBranchFromCommand("git checkout src/a.ts src/b.ts"), null);
  assert.equal(parseBranchFromCommand("git checkout deadbeefcafe"), null);
  assert.equal(parseBranchFromCommand("cargo build --release"), null);
  assert.equal(parseBranchFromCommand("echo git checkout"), null);
});

test("deriveChatWorkBranch returns the latest branch across the transcript", () => {
  const transcript = [
    toolItem("ls -la", { id: "1" }),
    toolItem("git worktree add ../wt -b first-branch", { id: "2" }),
    { id: "3", type: "lifecycle", renderClass: "status" },
    toolItem("git checkout -b second-branch", { id: "4" }),
    toolItem("cargo test", { id: "5" }),
  ];
  assert.equal(deriveChatWorkBranch(transcript), "second-branch");
});

test("deriveChatWorkBranchSource returns the latest branch timestamp", () => {
  const transcript = [
    toolItem("git worktree add ../wt -b first-branch", {
      id: "1",
      timestamp: "2026-07-04T00:00:01Z",
    }),
    toolItem("git checkout -b second-branch", {
      id: "2",
      timestamp: "2026-07-04T00:00:03Z",
    }),
  ];

  assert.deepEqual(deriveChatWorkBranchSource(transcript), {
    branch: "second-branch",
    timestampMs: Date.parse("2026-07-04T00:00:03Z"),
  });
  assert.deepEqual(collectChatWorkBranchSources(transcript), [
    {
      branch: "first-branch",
      timestampMs: Date.parse("2026-07-04T00:00:01Z"),
    },
    {
      branch: "second-branch",
      timestampMs: Date.parse("2026-07-04T00:00:03Z"),
    },
  ]);
});

test("deriveChatWorkBranch is null without branch activity", () => {
  assert.equal(deriveChatWorkBranch([toolItem("pnpm test")]), null);
});

const AGENT_PK = "cd".repeat(32);

test("agent messages announcing a worktree branch are parsed", () => {
  const messages = [
    {
      pubkey: "ff".repeat(32),
      content: "please make a worktree",
      created_at: 100,
    },
    {
      pubkey: AGENT_PK,
      content:
        "Done! Created a new worktree at /Users/k/Development/sprout-dictation " +
        "on branch kennylopez-dictation, based off latest main.",
      created_at: 200,
    },
  ];
  assert.equal(
    deriveBranchFromAgentMessages(messages, AGENT_PK),
    "kennylopez-dictation",
  );
  assert.deepEqual(deriveBranchSourceFromAgentMessages(messages, AGENT_PK), {
    branch: "kennylopez-dictation",
    timestampMs: 200_000,
  });
  assert.deepEqual(collectBranchSourcesFromAgentMessages(messages, AGENT_PK), [
    {
      branch: "kennylopez-dictation",
      timestampMs: 200_000,
    },
  ]);
});

test("backticked branch names and quoted commands in messages parse too", () => {
  assert.equal(
    deriveBranchFromAgentMessages(
      [{ pubkey: AGENT_PK, content: "I pushed the branch `fix/panel-width`." }],
      AGENT_PK,
    ),
    "fix/panel-width",
  );
  assert.equal(
    deriveBranchFromAgentMessages(
      [{ pubkey: AGENT_PK, content: "Ran `git checkout -b quick-fix` first." }],
      AGENT_PK,
    ),
    "quick-fix",
  );
});

test("agent messages mentioning multiple branches are ignored as ambiguous", () => {
  assert.equal(
    deriveBranchFromAgentMessages(
      [
        {
          pubkey: AGENT_PK,
          content:
            "Spellcheck is on branch `feat/spellcheck`; dictation is on branch `kennylopez-dictation`.",
        },
      ],
      AGENT_PK,
    ),
    null,
  );
});

test("ambiguous branch summaries do not overwrite an earlier concrete branch", () => {
  const messages = [
    {
      pubkey: AGENT_PK,
      content:
        "Created a new worktree at /Users/k/Development/sprout-spellcheck on branch feat/spellcheck.",
    },
    {
      pubkey: AGENT_PK,
      content:
        "Spellcheck is on branch `feat/spellcheck`; dictation is on branch `kennylopez-dictation`.",
    },
  ];

  assert.equal(
    deriveBranchFromAgentMessages(messages, AGENT_PK),
    "feat/spellcheck",
  );
});

test("non-agent messages and branchless text derive nothing", () => {
  assert.equal(
    deriveBranchFromAgentMessages(
      [
        { pubkey: "ff".repeat(32), content: "on branch user-branch" },
        { pubkey: AGENT_PK, content: "All tests pass now." },
      ],
      AGENT_PK,
    ),
    null,
  );
  assert.equal(deriveBranchFromAgentMessages([], null), null);
});

test("placeholder tokens in template commands never parse as branches", () => {
  assert.equal(parseBranchFromCommand("git checkout -b <branch>"), null);
  assert.equal(parseBranchFromCommand("git switch <name>"), null);
  assert.equal(parseBranchFromCommand("git worktree add ../wt <branch>"), null);
  assert.equal(parseBranchFromCommand("git worktree add ../<dir>"), null);
  assert.equal(
    deriveBranchFromAgentMessages(
      [
        {
          pubkey: "cd".repeat(32),
          content: "Run `git checkout -b <branch>` to start.",
        },
      ],
      "cd".repeat(32),
    ),
    null,
  );
});
