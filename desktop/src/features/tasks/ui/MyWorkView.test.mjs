import assert from "node:assert/strict";
import { after, before, describe, it } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  document: dom.window.document,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;

const ME = "a".repeat(64);
const OTHER = "b".repeat(64);
const TASKS = [
  {
    id: "needs-me",
    channelId: "construct",
    title: "Approve the equipment map direction",
    body: "Use telematics positions first and a manual pin only as fallback.",
    status: "open",
    assignee: ME,
    createdBy: OTHER,
    priority: 4,
    source: "telegram",
    sourceRef: "telegram:thread:42",
    dueAt: null,
    doneAt: null,
    createdAt: 10,
    updatedAt: 40,
  },
  {
    id: "running",
    channelId: "lab",
    title: "Run the routing experiment",
    body: null,
    status: "in_progress",
    assignee: OTHER,
    createdBy: ME,
    priority: 2,
    source: "hermes",
    sourceRef: "session:abc",
    dueAt: null,
    doneAt: null,
    createdAt: 20,
    updatedAt: 30,
  },
  {
    id: "queued",
    channelId: "construct",
    title: "Review the Procon upload",
    body: null,
    status: "open",
    assignee: null,
    createdBy: ME,
    priority: 1,
    source: "telegram",
    sourceRef: null,
    dueAt: null,
    doneAt: null,
    createdAt: 15,
    updatedAt: 20,
  },
  {
    id: "done",
    channelId: "buzz",
    title: "Ship channel task views",
    body: null,
    status: "done",
    assignee: OTHER,
    createdBy: ME,
    priority: 0,
    source: "app",
    sourceRef: null,
    dueAt: null,
    doneAt: 50,
    createdAt: 5,
    updatedAt: 50,
  },
];

let React, act, createRoot, MyWorkView;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ MyWorkView } = await import("./MyWorkView.tsx"));
});

after(() => dom.window.close());

async function mount(props = {}) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const calls = [];
  await act(async () => {
    root.render(
      React.createElement(MyWorkView, {
        tasks: TASKS,
        currentPubkey: ME,
        channelNamesById: new Map([
          ["construct", "equipment-control"],
          ["lab", "inference-lab"],
          ["buzz", "buzz"],
        ]),
        isLoading: false,
        error: null,
        isStatusPending: false,
        onRetry: () => calls.push(["retry"]),
        onSetStatus: (task, status) => calls.push([task.id, status]),
        ...props,
      }),
    );
  });
  return { calls, container, root };
}

async function unmount(view) {
  await act(async () => view.root.unmount());
  view.container.remove();
}

describe("MyWorkView", () => {
  it("groups real tasks into intervention-first queue sections", async () => {
    const view = await mount();
    assert.equal(
      view.container.querySelectorAll(
        '[data-testid="my-work-needs-you"] [data-work-id]',
      ).length,
      1,
    );
    assert.equal(
      view.container.querySelectorAll(
        '[data-testid="my-work-in-progress"] [data-work-id]',
      ).length,
      1,
    );
    assert.equal(
      view.container.querySelectorAll(
        '[data-testid="my-work-queued"] [data-work-id]',
      ).length,
      1,
    );
    assert.equal(
      view.container.querySelectorAll(
        '[data-testid="my-work-done"] [data-work-id]',
      ).length,
      1,
    );
    assert.equal(
      view.container.querySelector('[data-testid="my-work-detail-title"]')
        .textContent,
      "Approve the equipment map direction",
      "the highest-intervention task is selected first",
    );
    await unmount(view);
  });

  it("selects a queue row and shows only real task detail fields", async () => {
    const view = await mount();
    await act(async () => {
      view.container.querySelector('[data-work-id="running"]').click();
    });
    assert.equal(
      view.container.querySelector('[data-testid="my-work-detail-title"]')
        .textContent,
      "Run the routing experiment",
    );
    assert.match(
      view.container.querySelector('[data-testid="my-work-detail-source"]')
        .textContent,
      /hermes/,
    );
    assert.equal(
      view.container.querySelector('[data-testid="my-work-request-body"]'),
      null,
      "a missing body must not be replaced with invented request prose",
    );
    await unmount(view);
  });

  it("keeps relay failure distinct from an empty queue and exposes retry", async () => {
    const errored = await mount({
      tasks: [],
      error: new Error("task API unavailable"),
    });
    assert.match(
      errored.container.querySelector('[data-testid="my-work-error"]')
        .textContent,
      /task API unavailable/,
    );
    await act(async () => {
      errored.container.querySelector('[data-testid="my-work-retry"]').click();
    });
    assert.deepEqual(errored.calls, [["retry"]]);
    assert.equal(
      errored.container.querySelector('[data-testid="my-work-empty"]'),
      null,
    );
    await unmount(errored);

    const empty = await mount({ tasks: [] });
    assert.ok(empty.container.querySelector('[data-testid="my-work-empty"]'));
    assert.equal(
      empty.container.querySelector('[data-testid="my-work-error"]'),
      null,
    );
    await unmount(empty);
  });

  it("routes status actions through the canonical mutation callback", async () => {
    const view = await mount();
    await act(async () => {
      view.container
        .querySelector('[data-testid="my-work-start-task"]')
        .click();
    });
    assert.deepEqual(view.calls, [["needs-me", "in_progress"]]);
    await unmount(view);
  });
});
