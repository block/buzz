import assert from "node:assert/strict";
import test from "node:test";

import { projectPullRequestConflictCommands } from "./projectPullRequestConflictRecovery.ts";

test("builds copyable commands without executing merge recovery", () => {
  assert.deepEqual(
    projectPullRequestConflictCommands({
      recoveryRef: `refs/buzz/merge-recovery/${"a".repeat(40)}`,
      targetBranch: "main",
      targetRef: `refs/buzz/merge-recovery-target/${"b".repeat(40)}`,
    }),
    [
      'test -z "$(git status --porcelain=v1)" &&',
      `(git switch 'main' || git switch --create 'main' 'refs/buzz/merge-recovery-target/${"b".repeat(40)}') &&`,
      `git merge-base --is-ancestor HEAD 'refs/buzz/merge-recovery-target/${"b".repeat(40)}' &&`,
      `git merge --ff-only 'refs/buzz/merge-recovery-target/${"b".repeat(40)}' &&`,
      `git merge 'refs/buzz/merge-recovery/${"a".repeat(40)}'`,
    ],
  );
});

test("quotes recovery values as inert shell arguments", () => {
  const commands = projectPullRequestConflictCommands({
    recoveryRef: `refs/buzz/merge-recovery/${"b".repeat(40)}`,
    targetBranch: "release candidate",
    targetRef: `refs/buzz/merge-recovery-target/${"c".repeat(40)}`,
  });

  assert.equal(
    commands[1],
    `(git switch 'release candidate' || git switch --create 'release candidate' 'refs/buzz/merge-recovery-target/${"c".repeat(40)}') &&`,
  );
  assert.equal(
    commands[4],
    `git merge 'refs/buzz/merge-recovery/${"b".repeat(40)}'`,
  );
});
