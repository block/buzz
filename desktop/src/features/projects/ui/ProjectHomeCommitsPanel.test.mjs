import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

after(() => dom.window.close());

function repository(id, name) {
  return {
    id,
    name,
    repoAddress: `30617:owner:${id}`,
    defaultBranch: "main",
  };
}

test("multi-repository commits remain visibly degraded when one repository fails", async () => {
  const { cleanup, render, screen } = await import("@testing-library/react");
  const { ProjectHomeCommitsPanel } = await import(
    "./ProjectHomeCommitsPanel.tsx"
  );
  const loadedRepository = repository("loaded", "Loaded");
  const failedRepository = repository("failed", "Failed");

  const React = await import("react");
  try {
    render(
      React.createElement(ProjectHomeCommitsPanel, {
        onSelectCommit: () => {},
        projectId: "project-1",
        pullRequests: [],
        results: [
          {
            error: null,
            isLoading: false,
            repository: loadedRepository,
            snapshot: {
              contributors: [],
              commits: [
                {
                  hash: "a".repeat(40),
                  shortHash: "aaaaaaa",
                  authorName: "Alice",
                  authorEmail: "alice@example.com",
                  timestamp: 2,
                  subject: "Loaded commit",
                },
              ],
            },
          },
          {
            error: new Error("unavailable"),
            isLoading: false,
            repository: failedRepository,
            snapshot: null,
          },
        ],
      }),
    );

    assert.match(
      screen.getByTestId("project-home-commits-degraded").textContent,
      /Showing commits from 1 of 2 repositories/,
    );
    assert.match(document.body.textContent, /Loaded commit/);
  } finally {
    cleanup();
  }
});

test("multi-repository commits are merged in descending timestamp order", async () => {
  const { cleanup, render } = await import("@testing-library/react");
  const { ProjectHomeCommitsPanel } = await import(
    "./ProjectHomeCommitsPanel.tsx"
  );
  const React = await import("react");
  const result = (id, subject, timestamp) => ({
    error: null,
    isLoading: false,
    repository: repository(id, id),
    snapshot: {
      contributors: [],
      commits: [
        {
          hash: id.repeat(40),
          shortHash: id.repeat(7),
          authorName: id,
          authorEmail: `${id}@example.com`,
          timestamp,
          subject,
        },
      ],
    },
  });

  try {
    render(
      React.createElement(ProjectHomeCommitsPanel, {
        onSelectCommit: () => {},
        projectId: "project-1",
        pullRequests: [],
        results: [
          result("a", "Older commit", 1),
          result("b", "Newer commit", 2),
        ],
      }),
    );

    assert.ok(
      document.body.textContent.indexOf("Newer commit") <
        document.body.textContent.indexOf("Older commit"),
    );
  } finally {
    cleanup();
  }
});
