import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

import { buildProjectDetailAgentContext } from "../lib/projectDetailAgentContext.ts";
import { projectDetailSelectionItem } from "../lib/projectDetailSelectionItem.ts";
import { reviewDiffWorkspaceBranch } from "../lib/projectReviewDisplay.ts";
import { pullRequestsPanelKind } from "./PullRequestsPanelSurface.tsx";
import { buildProjectDetailCrumbs } from "./useProjectDetailCrumbs.ts";

// The real composer mounts TipTap and never releases jsdom handles. Stub it so
// the production panel can prove selectedPullRequest wiring without hanging
// the node:test process.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "@/features/forum/ui/ForumComposer") {
      return { shortCircuit: true, url: "buzz-pr-panel-stub:ForumComposer" };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-pr-panel-stub:ForumComposer") {
      return {
        format: "module",
        shortCircuit: true,
        source:
          "globalThis.__FORUM_COMPOSER_STUBBED__ = true;\nexport function ForumComposer() { return null; }\n",
      };
    }
    return nextLoad(url, context);
  },
});

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
  channelId: null,
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

function assertRetainedReviewDetail(screen) {
  const detail = screen.getByTestId("project-pull-request-detail");
  const title = detail.querySelector("h3");
  assert.ok(title, "real PullRequestsPanel detail should render an h3 title");
  assert.match(title.textContent, /Ship the retained review/);
  assert.match(
    detail.textContent,
    /Keep this description visible while the list refetches/,
  );
  assert.equal(screen.queryByTestId("project-pull-requests-empty"), null);
  assert.equal(screen.queryByText("No reviews yet."), null);
}

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

class NoopObserver {
  disconnect() {}
  observe() {}
  unobserve() {}
}

Object.assign(globalThis, {
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  IntersectionObserver: NoopObserver,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: NoopObserver,
  document: dom.window.document,
  getComputedStyle: (...args) => dom.window.getComputedStyle(...args),
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
  writable: true,
});
Object.defineProperty(dom.window.navigator, "mediaDevices", {
  configurable: true,
  value: {
    addEventListener: () => {},
    enumerateDevices: async () => [],
    removeEventListener: () => {},
  },
});
dom.window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
dom.window.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;
const tauriInternals = {
  invoke: async (cmd) => {
    if (cmd === "get_identity") return { pubkey: OWNER };
    if (cmd === "search_messages") return { found: 0, hits: [] };
    if (cmd === "get_open_channel_directory") return [];
    if (cmd === "get_channel_details") return null;
    if (cmd === "list_channels") return [];
    if (cmd === "get_users_batch") return { missing: [], profiles: {} };
    if (cmd.startsWith("plugin:event|")) return 0;
    throw new Error(`unmocked Tauri command: ${cmd}`);
  },
  transformCallback: () => 1,
};
globalThis.__TAURI_INTERNALS__ = tauriInternals;
dom.window.__TAURI_INTERNALS__ = tauriInternals;
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ =
  globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__;

after(() => dom.window.close());

const workspaceTabsStub = {
  commitDiff: null,
  commitDiffError: null,
  commitDiffLoading: false,
  contributorActivityCounts: {},
  contributorPubkeys: [],
  createIssueAction: { onCreate: async () => {}, pending: false },
  localSnapshot: null,
  localSnapshotError: null,
  localSnapshotLoading: false,
  onSelectedCommitHashChange: noop,
  onSelectedIssueIdChange: noop,
  onSelectedPullRequestIdChange: noop,
  projectId: "project-id",
  repoContributors: [],
  repoDiff: null,
  repoDiffError: null,
  repoDiffLoading: false,
  repoHost: { kind: "unresolved" },
  repoSource: "remote",
  selectedCommitHash: null,
  selectedIssueId: null,
  snapshot: null,
  snapshotError: null,
  snapshotLoading: false,
};

let React;
let act;
let createRoot;
let hookModule;
let workspaceTabsModule;
let QueryClient;
let QueryClientProvider;
let CommunitiesProvider;
let HuddleProvider;
let TooltipProvider;
let createMemoryHistory;
let createRootRoute;
let createRoute;
let createRouter;
let RouterProvider;
let channelsQueryKey;
before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  hookModule = await import("./useRetainedProjectGitViews.ts");
  workspaceTabsModule = await import("./ProjectWorkspaceTabs.tsx");
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ HuddleProvider } = await import("@/features/huddle"));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip"));
  ({
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    RouterProvider,
  } = await import("@tanstack/react-router"));
  ({ channelsQueryKey } = await import("@/features/channels/hooks.ts"));
});

async function renderSelection(initialProps) {
  let props = initialProps;
  const result = { current: null };
  function Probe() {
    result.current = hookModule.useRetainedPullRequestSelection(props);
    return null;
  }
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(React.createElement(Probe));
  });
  return {
    result,
    async rerender(nextProps) {
      props = nextProps;
      await act(async () => {
        root.render(React.createElement(Probe));
      });
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
      container.remove();
    },
  };
}

async function renderWorkspaceReviews(initialProps) {
  let setPanelProps = noop;
  const { WorkspaceTabs } = workspaceTabsModule;
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  client.setQueryData(["identity"], { pubkey: OWNER });
  client.setQueryData(channelsQueryKey, []);
  function Panel() {
    const [props, setProps] = React.useState(initialProps);
    setPanelProps = setProps;
    return React.createElement(WorkspaceTabs, {
      ...workspaceTabsStub,
      initialTab: "prs",
      project: repository,
      pullRequests: props.pullRequests,
      pullRequestsError: null,
      pullRequestsLoading: props.isLoading,
      selectedPullRequest: props.selectedPullRequest,
      selectedPullRequestId: props.selectedPullRequestId,
    });
  }
  const rootRoute = createRootRoute({ component: Panel });
  const channelRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/channels/$channelId",
    component: () => null,
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute.addChildren([channelRoute]),
  });
  await router.load();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const tree = () =>
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            HuddleProvider,
            null,
            React.createElement(RouterProvider, { router }),
          ),
        ),
      ),
    );
  await act(async () => {
    root.render(tree());
  });
  return {
    async rerender(nextProps) {
      await act(async () => {
        setPanelProps(nextProps);
      });
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
      client.clear();
      client.unmount();
      container.remove();
    },
  };
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
  const {
    rerender,
    result,
    unmount: unmountSelection,
  } = await renderSelection(populated);

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
  // Drive the production WorkspaceTabs → PullRequestsPanel handoff
  // (ProjectWorkspaceTabs.tsx selectedPullRequest={selectedPullRequest}).
  // Hardcoding that prop to null makes the fetching-empty assertions fail.
  const panel = await renderWorkspaceReviews({
    isLoading: false,
    pullRequests: populated.pullRequests,
    selectedPullRequest: result.current.selectedPullRequest,
    selectedPullRequestId: REVIEW_A_ID,
  });
  try {
    assert.equal(globalThis.__FORUM_COMPOSER_STUBBED__, true);
    assertRetainedReviewDetail(screen);

    await rerender({
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
    await panel.rerender({
      isLoading: false,
      pullRequests: [],
      selectedPullRequest: result.current.selectedPullRequest,
      selectedPullRequestId: REVIEW_A_ID,
    });
    assertRetainedReviewDetail(screen);

    await rerender({
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
    await panel.rerender({
      isLoading: false,
      pullRequests: [],
      selectedPullRequest: result.current.selectedPullRequest,
      selectedPullRequestId: REVIEW_A_ID,
    });
    assert.match(
      screen.getByTestId("project-pull-requests-empty").textContent,
      /^No reviews yet/,
    );
    assert.equal(screen.queryByTestId("project-pull-request-detail"), null);
  } finally {
    await panel.unmount();
    await unmountSelection();
  }
});
