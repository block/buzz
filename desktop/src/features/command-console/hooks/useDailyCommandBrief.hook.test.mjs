import assert from "node:assert/strict";
import { mock } from "node:test";
import test from "node:test";

import { installHookTestDom } from "./hookTestDom.mjs";

installHookTestDom();

const React = await import("react");
const { act } = React;
const { createRoot } = await import("react-dom/client");
const { useDailyCommandBrief } = await import("./useDailyCommandBrief.ts");

const status = (state, overrides = {}) => ({
  classification: "OFFICIAL",
  runId: "run-1",
  scheduleId: "daily-command-brief",
  state,
  updatedAt: "2026-07-25T06:00:00Z",
  degradedSections: [],
  error: null,
  ...overrides,
});

const schedule = {
  classification: "OFFICIAL",
  scheduleId: "daily-command-brief",
  enabled: true,
  localTime: "06:00",
  timezone: "Australia/Sydney",
  catchUpSameDay: true,
  concurrency: 1,
};

function renderHook(useValue) {
  let value;
  const root = createRoot(document.createElement("div"));
  function Harness() {
    value = useValue();
    return null;
  }
  return {
    get value() {
      return value;
    },
    async mount() {
      await act(async () => root.render(React.createElement(Harness)));
    },
    async unmount() {
      await act(async () => root.unmount());
    },
  };
}

test("loads the immutable status, latest brief, and schedule then follows metadata events", async () => {
  let listener;
  const deps = {
    getStatus: mock.fn(async () => ({
      classification: "OFFICIAL",
      current: null,
      history: [],
    })),
    getLatest: mock.fn(async () => null),
    getSchedule: mock.fn(async () => schedule),
    start: mock.fn(async () => status("queued")),
    cancel: mock.fn(async () => status("cancelled")),
    setSchedule: mock.fn(async () => ({ ...schedule, concurrency: 2 })),
    subscribeStatus: mock.fn(async (next) => {
      listener = next;
      return () => {};
    }),
  };
  const hook = renderHook(() => useDailyCommandBrief(deps));

  try {
    await hook.mount();
    assert.equal(hook.value.loading, false);
    assert.equal(hook.value.latest, null);
    assert.equal(hook.value.schedule.concurrency, 1);

    await act(async () => {
      await hook.value.start();
    });
    assert.equal(hook.value.status.state, "queued");

    await act(async () => {
      listener(status("queued"));
    });
    assert.equal(hook.value.history.length, 1);

    await act(async () => {
      listener(status("running_specialists"));
    });
    assert.equal(hook.value.status.state, "running_specialists");

    await act(async () => {
      await hook.value.cancel();
    });
    assert.equal(hook.value.status.state, "cancelled");

    await act(async () => {
      await hook.value.updateSchedule({
        enabled: true,
        localTime: "06:00",
        concurrency: 2,
      });
    });
    assert.equal(hook.value.schedule.concurrency, 2);
  } finally {
    await hook.unmount();
  }
});

test("refresh failure is redacted for display and does not discard a prior brief", async () => {
  const latest = Object.freeze({ marker: "validated immutable brief" });
  let fail = false;
  const deps = {
    getStatus: async () => ({
      classification: "OFFICIAL",
      current: status("completed"),
      history: [status("completed")],
    }),
    getLatest: async () => {
      if (fail) throw new Error("bearer secret provider body");
      return latest;
    },
    getSchedule: async () => schedule,
    start: async () => status("queued"),
    cancel: async () => status("cancelled"),
    setSchedule: async () => schedule,
    subscribeStatus: async () => () => {},
  };
  const hook = renderHook(() => useDailyCommandBrief(deps));

  try {
    await hook.mount();
    assert.equal(hook.value.latest, latest);
    fail = true;
    await act(async () => {
      await hook.value.refresh();
    });
    assert.equal(hook.value.latest, latest);
    assert.equal(hook.value.error, "Daily Command Brief is unavailable.");
    assert.doesNotMatch(hook.value.error, /secret|provider|body/i);
  } finally {
    await hook.unmount();
  }
});
