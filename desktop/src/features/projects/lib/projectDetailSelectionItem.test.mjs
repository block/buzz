import assert from "node:assert/strict";
import test from "node:test";

import { projectDetailSelectionItem } from "./projectDetailSelectionItem.ts";

const repository = {
  channelId: "trusted-repository-channel",
};

test("detail work items ignore author-claimed origin channels", () => {
  const issue = projectDetailSelectionItem({
    issue: {
      author: "issue-author",
      channelId: "forged-origin-channel",
      id: "issue-id",
      repoAddress: null,
      title: "Forged origin task",
    },
    projectChannelId: "trusted-project-channel",
    projectId: "project-id",
    repository,
  });
  const pullRequest = projectDetailSelectionItem({
    projectChannelId: "trusted-project-channel",
    projectId: "project-id",
    pullRequest: {
      author: "review-author",
      channelId: "forged-origin-channel",
      id: "review-id",
      repoAddress: null,
      title: "Forged origin review",
    },
    repository,
  });

  assert.equal(issue?.channelId, "trusted-repository-channel");
  assert.equal(pullRequest?.channelId, "trusted-repository-channel");
});

test("detail items fall back to the trusted project channel", () => {
  const item = projectDetailSelectionItem({
    issue: {
      author: "issue-author",
      channelId: "forged-origin-channel",
      id: "issue-id",
      repoAddress: null,
      title: "Forged origin task",
    },
    projectChannelId: "trusted-project-channel",
    projectId: "project-id",
    repository: { ...repository, channelId: null },
  });

  assert.equal(item?.channelId, "trusted-project-channel");
});
