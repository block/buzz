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
  sequence: 0,
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
    cancel: mock.fn(async () => status("cancelled", { sequence: 2 })),
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
      listener(status("running_specialists", { sequence: 1 }));
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

test("subscribes before initial reconciliation and retains fast ordered transitions", async () => {
  const calls = [];
  const initial = Promise.withResolvers();
  let listener;
  const deps = {
    getStatus: async () => {
      calls.push("status");
      return initial.promise;
    },
    getLatest: async () => null,
    getSchedule: async () => schedule,
    start: async () => status("queued"),
    cancel: async () => status("cancelled", { sequence: 3 }),
    setSchedule: async () => schedule,
    subscribeStatus: async (next) => {
      calls.push("subscribe");
      listener = next;
      return () => {};
    },
  };
  const hook = renderHook(() => useDailyCommandBrief(deps));
  try {
    await hook.mount();
    assert.deepEqual(calls.slice(0, 2), ["subscribe", "status"]);

    await act(async () => {
      listener(status("collecting_sources", { sequence: 1 }));
      listener(status("running_specialists", { sequence: 2 }));
    });
    await act(async () => {
      initial.resolve({
        classification: "OFFICIAL",
        current: status("queued"),
        history: [status("queued")],
      });
      await initial.promise;
    });

    assert.equal(hook.value.status.sequence, 2);
    assert.deepEqual(
      hook.value.history.map((entry) => entry.sequence),
      [0, 1, 2],
    );

    await act(async () => {
      listener(status("failed", { sequence: 1 }));
      listener(status("failed", { sequence: 2 }));
      listener(
        status("queued", {
          runId: "unrelated-run",
          sequence: 0,
        }),
      );
    });
    assert.equal(hook.value.status.runId, "run-1");
    assert.equal(hook.value.status.sequence, 2);
    assert.equal(hook.value.status.state, "running_specialists");
  } finally {
    await hook.unmount();
  }
});

test("reverse start responses cannot replace the newest coherent run", async () => {
  let listener;
  const first = Promise.withResolvers();
  const second = Promise.withResolvers();
  let startCount = 0;
  const deps = {
    getStatus: async () => ({
      classification: "OFFICIAL",
      current: null,
      history: [],
    }),
    getLatest: async () => null,
    getSchedule: async () => schedule,
    start: async () => (++startCount === 1 ? first.promise : second.promise),
    cancel: async () => status("cancelled"),
    setSchedule: async () => schedule,
    subscribeStatus: async (next) => {
      listener = next;
      return () => {};
    },
  };
  const hook = renderHook(() => useDailyCommandBrief(deps));
  try {
    await hook.mount();
    let firstStart;
    let secondStart;
    await act(async () => {
      firstStart = hook.value.start();
      secondStart = hook.value.start();
    });
    second.resolve(
      status("queued", {
        runId: "run-new",
      }),
    );
    await act(async () => {
      await secondStart;
    });
    await act(async () => {
      listener(
        status("collecting_sources", {
          runId: "run-new",
          sequence: 1,
        }),
      );
    });
    first.resolve(
      status("queued", {
        runId: "run-old",
      }),
    );
    await act(async () => {
      await firstStart;
    });

    assert.equal(hook.value.status.runId, "run-new");
    assert.equal(hook.value.status.sequence, 1);
    assert.deepEqual(
      hook.value.history.map((entry) => entry.runId),
      ["run-new", "run-new"],
    );
  } finally {
    await hook.unmount();
  }
});

test("reverse terminal latest promises cannot mix briefs from different runs", async () => {
  let listener;
  const latestOne = Promise.withResolvers();
  const latestTwo = Promise.withResolvers();
  let latestCount = 0;
  const deps = {
    getStatus: async () => ({
      classification: "OFFICIAL",
      current: null,
      history: [],
    }),
    getLatest: async () => {
      latestCount += 1;
      if (latestCount === 1) return null;
      return latestCount === 2 ? latestOne.promise : latestTwo.promise;
    },
    getSchedule: async () => schedule,
    start: async () => status("queued"),
    cancel: async () => status("cancelled"),
    setSchedule: async () => schedule,
    subscribeStatus: async (next) => {
      listener = next;
      return () => {};
    },
  };
  const hook = renderHook(() => useDailyCommandBrief(deps));
  try {
    await hook.mount();
    await act(async () => {
      listener(status("completed", { runId: "run-one", sequence: 3 }));
      listener(status("queued", { runId: "run-two", sequence: 0 }));
      listener(status("completed", { runId: "run-two", sequence: 2 }));
    });
    await act(async () => {
      latestTwo.resolve({ brief: { runId: "run-two" } });
      await latestTwo.promise;
    });
    await act(async () => {
      latestOne.resolve({ brief: { runId: "run-one" } });
      await latestOne.promise;
    });

    assert.equal(hook.value.status.runId, "run-two");
    assert.equal(hook.value.latest.brief.runId, "run-two");
  } finally {
    await hook.unmount();
  }
});

test("rapid schedule patches stay merged while serialized responses settle in either edit order", async () => {
  for (const patches of [
    [{ enabled: false }, { concurrency: 2 }],
    [{ concurrency: 2 }, { enabled: false }],
  ]) {
    const writes = [];
    const calls = [];
    const deps = {
      getStatus: async () => ({
        classification: "OFFICIAL",
        current: null,
        history: [],
      }),
      getLatest: async () => null,
      getSchedule: async () => schedule,
      start: async () => status("queued"),
      cancel: async () => status("cancelled"),
      setSchedule: async (update) => {
        calls.push(update);
        const pending = Promise.withResolvers();
        writes.push(pending);
        return pending.promise;
      },
      subscribeStatus: async () => () => {},
    };
    const hook = renderHook(() => useDailyCommandBrief(deps));
    try {
      await hook.mount();
      let firstUpdate;
      let secondUpdate;
      await act(async () => {
        firstUpdate = hook.value.updateSchedule(patches[0]);
        secondUpdate = hook.value.updateSchedule(patches[1]);
        await Promise.resolve();
      });

      assert.equal(hook.value.schedule.enabled, false);
      assert.equal(hook.value.schedule.concurrency, 2);
      assert.equal(writes.length, 1, "writes must be serialized");

      await act(async () => {
        writes[0].resolve({
          ...schedule,
          enabled: patches[0].enabled ?? schedule.enabled,
          concurrency: patches[0].concurrency ?? schedule.concurrency,
        });
        await writes[0].promise;
        await Promise.resolve();
      });
      assert.equal(writes.length, 2);
      assert.deepEqual(calls[1], {
        enabled: false,
        localTime: "06:00",
        concurrency: 2,
      });
      assert.equal(hook.value.schedule.enabled, false);
      assert.equal(hook.value.schedule.concurrency, 2);

      await act(async () => {
        writes[1].resolve({ ...schedule, enabled: false, concurrency: 2 });
        await Promise.all([firstUpdate, secondUpdate]);
      });
      assert.equal(hook.value.schedule.enabled, false);
      assert.equal(hook.value.schedule.concurrency, 2);
    } finally {
      await hook.unmount();
    }
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

test("a successfully loaded terminal brief clears an earlier unavailable banner", async () => {
  let listener;
  let latestAttempts = 0;
  const published = Object.freeze({ brief: { runId: "run-1" } });
  const deps = {
    getStatus: async () => ({
      classification: "OFFICIAL",
      current: null,
      history: [],
    }),
    getLatest: async () => {
      latestAttempts += 1;
      if (latestAttempts === 1) throw new Error("temporary store contention");
      return published;
    },
    getSchedule: async () => schedule,
    start: async () => status("queued"),
    cancel: async () => status("cancelled"),
    setSchedule: async () => schedule,
    subscribeStatus: async (next) => {
      listener = next;
      return () => {};
    },
  };
  const hook = renderHook(() => useDailyCommandBrief(deps));

  try {
    await hook.mount();
    assert.equal(hook.value.error, "Daily Command Brief is unavailable.");

    await act(async () => {
      listener(status("degraded", { sequence: 4 }));
      await Promise.resolve();
    });

    assert.equal(hook.value.latest, published);
    assert.equal(hook.value.error, null);
  } finally {
    await hook.unmount();
  }
});
