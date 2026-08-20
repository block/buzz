import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

import { buildProjectDetailAgentContext } from "../lib/projectDetailAgentContext.ts";
import { projectDetailSelectionItem } from "../lib/projectDetailSelectionItem.ts";
import { reviewDiffWorkspaceBranch } from "../lib/projectReviewDisplay.ts";
import { pullRequestsPanelKind } from "./PullRequestsPanelSurface.tsx";
import { buildProjectDetailCrumbs } from "./useProjectDetailCrumbs.ts";

const OWNER = "a".repeat(64);
const REVIEW_A_ID = "b".repeat(64);

const repository = {
  id: `${OWNER}:buzz`,
  dtag: "buzz",
  name: "buzz",
  description: "",
  cloneUrls: ["https://example.com/buzz.git"],
  webUrl: null,
  owner: OWNER,
  contributors: [],
  createdAt: 0,
  status: "open",
  defaultBranch: "main",
  repoAddress: `30617:${OWNER}:buzz`,
  channelId: "trusted-repository-channel",
};

const reviewA = {
  id: REVIEW_A_ID,
  title: "Ship the retained review",
  content: "Keep this description visible while the list refetches.",
  tags: [],
  author: OWNER,
  createdAt: 1_700_000_000,
  repoAddress: repository.repoAddress,
  channelId: "forged-origin-channel",
  originAgentName: null,
  labels: [],
  recipients: [],
  reviewers: [],
  approvals: [],
  changeRequests: [],
  status: "Open",
  statusEventId: null,
  statusCreatedAt: null,
  branchName: "feature-a",
  targetBranch: "main",
  initialCommit: null,
  commit: null,
  cloneUrls: repository.cloneUrls,
  updateCount: 0,
  updatedAt: 1_700_000_000,
  updates: [],
  comments: [],
};

const noop = () => {};

function productionConsumers({ activeRepoPullRequest, selectedPullRequest }) {
  const crumbs = buildProjectDetailCrumbs({
    activeTab: "prs",
    commit: null,
    issue: null,
    pullRequest: selectedPullRequest,
    setRequestedTab: noop,
    setSelectedCommitHash: noop,
    setSelectedIssueId: noop,
    setSelectedPullRequestId: noop,
    setTabsResetKey: noop,
  });
  const contextItem = projectDetailSelectionItem({
    projectChannelId: "trusted-project-channel",
    projectId: "project-id",
    pullRequest: selectedPullRequest,
    repository,
  });
  const agent = buildProjectDetailAgentContext({
    activeTab: "prs",
    branch: "feature-a",
    project: { name: "buzz" },
    repository: {
      name: repository.name,
      repoAddress: repository.repoAddress,
    },
    source: "remote",
    workItems: [null, null, selectedPullRequest],
  });
  return {
    agentReviewId: agent.workItem?.id ?? null,
    crumbTitle: crumbs.activeWorkItemCrumb?.title ?? null,
    contextId: contextItem?.id ?? null,
    diffQueryId: activeRepoPullRequest?.id ?? null,
    diffWorkspaceBranch: reviewDiffWorkspaceBranch({
      activeBranch: "feature-a",
      defaultBranch: repository.defaultBranch,
      pullRequest: activeRepoPullRequest,
    }),
  };
}

function panelConsumers({ isLoading, pullRequests, selectedPullRequest }) {
  return {
    kind: pullRequestsPanelKind({
      isLoading,
      pullRequests,
      selectedPullRequest,
    }),
    ...productionConsumers({
      activeRepoPullRequest: selectedPullRequest,
      selectedPullRequest,
    }),
  };
}

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
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

let hookModule;
let panelModule;
let surfaceModule;
before(async () => {
  hookModule = await import("./useRetainedProjectGitViews.ts");
  panelModule = await import("./ProjectPullRequestsPanel.tsx");
  surfaceModule = await import("./PullRequestsPanelSurface.tsx");
});

async function renderSelection(initialProps) {
  const { renderHook } = await import("@testing-library/react");
  return renderHook(
    (props) => hookModule.useRetainedPullRequestSelection(props),
    { initialProps },
  );
}

async function renderReviewsPanel({
  isLoading,
  pullRequests,
  selectedPullRequest,
}) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { PullRequestsPanel } = panelModule;
  const { PullRequestsPanelSurface } = surfaceModule;
  const tree = !selectedPullRequest
    ? createElement(PullRequestsPanel, {
        error: null,
        isLoading,
        onSelectedPullRequestIdChange: noop,
        project: repository,
        pullRequests,
        selectedPullRequest,
      })
    : createElement(PullRequestsPanelSurface, {
        detail: createElement(
          "div",
          { "data-testid": "project-pull-request-detail" },
          createElement("h3", null, selectedPullRequest.title),
          createElement("p", null, selectedPullRequest.content),
        ),
        error: null,
        isLoading,
        list: createElement("div", {
          "data-testid": "project-pull-requests-list",
        }),
        pullRequests,
        selectedPullRequest,
      });
  return render(tree);
}

test("selected review chrome and diff query stay aligned across fetch phases", async () => {
  const { screen } = await import("@testing-library/react");
  const populated = {
    activeBranch: "feature-a",
    isFetching: false,
    pullRequests: [reviewA],
    repository,
    selectedPullRequestId: REVIEW_A_ID,
  };
  const { rerender, result } = await renderSelection(populated);

  const populatedConsumers = panelConsumers({
    isLoading: false,
    pullRequests: populated.pullRequests,
    selectedPullRequest: result.current.selectedPullRequest,
  });
  assert.equal(result.current.selectedPullRequest, reviewA);
  assert.equal(result.current.activeRepoPullRequest, reviewA);
  assert.deepEqual(populatedConsumers, {
    agentReviewId: REVIEW_A_ID,
    crumbTitle: "Ship the retained review",
    contextId: `review:${REVIEW_A_ID}`,
    diffQueryId: REVIEW_A_ID,
    diffWorkspaceBranch: "main",
    kind: "detail",
  });
  let panel = await renderReviewsPanel({
    isLoading: false,
    pullRequests: populated.pullRequests,
    selectedPullRequest: result.current.selectedPullRequest,
  });
  assert.match(
    screen.getByRole("heading", { level: 3 }).textContent,
    /Ship the retained review/,
  );
  assert.match(
    screen.getByTestId("project-pull-request-detail").textContent,
    /Keep this description visible while the list refetches/,
  );
  assert.equal(screen.queryByTestId("project-pull-requests-empty"), null);

  rerender({
    ...populated,
    isFetching: true,
    pullRequests: [],
  });
  const fetchingConsumers = panelConsumers({
    isLoading: false,
    pullRequests: [],
    selectedPullRequest: result.current.selectedPullRequest,
  });
  assert.equal(result.current.selectedPullRequest, reviewA);
  assert.equal(
    result.current.selectedPullRequest,
    result.current.activeRepoPullRequest,
  );
  assert.deepEqual(fetchingConsumers, populatedConsumers);
  panel.unmount();
  panel = await renderReviewsPanel({
    isLoading: false,
    pullRequests: [],
    selectedPullRequest: result.current.selectedPullRequest,
  });
  assert.match(
    screen.getByRole("heading", { level: 3 }).textContent,
    /Ship the retained review/,
  );
  assert.match(
    screen.getByTestId("project-pull-request-detail").textContent,
    /Keep this description visible while the list refetches/,
  );
  assert.equal(screen.queryByText("No reviews yet."), null);

  rerender({
    ...populated,
    isFetching: false,
    pullRequests: [],
  });
  const completedConsumers = panelConsumers({
    isLoading: false,
    pullRequests: [],
    selectedPullRequest: result.current.selectedPullRequest,
  });
  assert.equal(result.current.selectedPullRequest, null);
  assert.equal(result.current.activeRepoPullRequest, null);
  assert.deepEqual(completedConsumers, {
    agentReviewId: null,
    crumbTitle: null,
    contextId: null,
    diffQueryId: null,
    diffWorkspaceBranch: "feature-a",
    kind: "empty",
  });
  panel.unmount();
  await renderReviewsPanel({
    isLoading: false,
    pullRequests: [],
    selectedPullRequest: result.current.selectedPullRequest,
  });
  assert.equal(
    screen.getByTestId("project-pull-requests-empty").textContent,
    "No reviews yet.",
  );
  assert.equal(screen.queryByTestId("project-pull-request-detail"), null);
});
