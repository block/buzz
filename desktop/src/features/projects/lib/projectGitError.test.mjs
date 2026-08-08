import assert from "node:assert/strict";
import { test } from "node:test";

import { projectCloneErrorPresentation } from "./projectGitError.ts";

test("explains unsupported authenticated GitHub clones without exposing git output", () => {
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
        "This repository requires GitHub authentication. Buzz currently clones public GitHub repositories without credentials.",
    },
  );
});

test("presents missing and network failures clearly", () => {
  assert.equal(
    projectCloneErrorPresentation(new Error("Repository not found")).title,
    "Repository not found",
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
