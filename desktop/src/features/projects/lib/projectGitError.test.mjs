import assert from "node:assert/strict";
import { test } from "node:test";

import { projectCloneErrorPresentation } from "./projectGitError.ts";

test("explains authenticated GitHub clone failures without exposing git output", () => {
  assert.deepEqual(
    projectCloneErrorPresentation(
      new Error(
        "Cloning into '/Users/person/repos/app'... remote: repository requires SSH certificate authentication. fatal: requested URL returned error: 403",
      ),
      "https://github.com/example/app.git",
    ),
    {
      title: "Repository access required",
      description:
        "This repository requires GitHub authentication. Sign in with the GitHub CLI and try again.",
    },
  );
});

test("explains authenticated GitLab clone failures with glab login guidance", () => {
  assert.deepEqual(
    projectCloneErrorPresentation(
      new Error(
        "Cloning into '/Users/person/repos/app'... remote: HTTP Basic: Access denied fatal: Authentication failed for 'https://gitlab.onlyarag.com/group/private-repo.git/'",
      ),
      "https://gitlab.onlyarag.com/group/private-repo.git",
    ),
    {
      title: "Repository access required",
      description:
        "This repository requires GitLab authentication. Sign in with the GitLab CLI (`glab auth login`) and try again.",
    },
  );
});

test("preserves missing credential helper remediation", () => {
  assert.deepEqual(
    projectCloneErrorPresentation(
      new Error(
        "fatal: could not read Username. If this repository is private, install the glab CLI, run `glab auth login`, and restart Buzz before trying again.",
      ),
      "https://gitlab.onlyarag.com/example/app.git",
    ),
    {
      title: "GitLab CLI required",
      description:
        "Install the GitLab CLI, run `glab auth login`, restart Buzz, and try again.",
    },
  );
});

test("presents missing and network failures clearly", () => {
  assert.equal(
    projectCloneErrorPresentation(new Error("Repository not found")).title,
    "Repository not found",
  );
  assert.deepEqual(
    projectCloneErrorPresentation(
      new Error("Repository not found"),
      "https://relay.example/git/owner/repo",
      "access",
    ),
    {
      title: "Repository access restricted",
      description:
        "You need access to the repository’s channel before you can clone it.",
    },
  );
  assert.equal(
    projectCloneErrorPresentation(new Error("Could not resolve host")).title,
    "Couldn’t reach the repository",
  );
});

test("uses a concise fallback", () => {
  assert.deepEqual(projectCloneErrorPresentation(new Error("git failed")), {
    title: "Couldn’t clone repository",
    description:
      "Try again. If the problem continues, contact the repository owner.",
  });
});
