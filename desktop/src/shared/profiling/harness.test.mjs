/**
 * Harness-core tests for the temporary renderer profiling branch (never merges).
 *
 * Scope is the pure, deterministic core only: ring-buffer bounds + flush
 * draining, drift-sampler math, and the JSONL record schema. The DOM/timer/IPC
 * probes are integration glue exercised by Will's day of driving, not unit
 * tests.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  classifyInput,
  computeDrift,
  DRIFT_THRESHOLD_MS,
  nextStallSample,
} from "@/shared/profiling/drift.ts";
import {
  createInvokeObserver,
  getInvokeObserver,
  setInvokeObserver,
  wrapInvoke,
} from "@/shared/profiling/ipc.ts";
import { ProfileRecorder, RING_CAPACITY } from "@/shared/profiling/recorder.ts";
import { resolveTauriCore } from "@/shared/profiling/viteTauriCoreProxy.ts";

function fixedClock() {
  let t = 1000;
  return {
    now: () => (t += 1),
    wall: () => 1_700_000_000_000,
  };
}

describe("computeDrift", () => {
  it("returns null when the fire is within threshold", () => {
    assert.equal(computeDrift(1050, 1000, 100), null);
    // Exactly at threshold is not a stall (strictly greater).
    assert.equal(computeDrift(1000 + DRIFT_THRESHOLD_MS, 1000), null);
  });

  it("returns the overage when the timer fires late past threshold", () => {
    assert.equal(computeDrift(1200, 1000, 50), 200);
  });

  it("treats early fires as no stall", () => {
    assert.equal(computeDrift(980, 1000, 50), null);
  });
});

describe("nextStallSample", () => {
  it("records a real overrun in an eligible (foreground) interval", () => {
    // armed at 1000, interval 500 → expected 1500; observed 1800 → 300 overrun.
    const { dur, armedAt } = nextStallSample(1000, 1800, true, 500, 50);
    assert.equal(dur, 300);
    assert.equal(armedAt, 1800);
  });

  it("suppresses the record but re-arms the baseline when ineligible", () => {
    // A hidden/blurred interval defers the timer arbitrarily; the huge gap must
    // not be scored, but armedAt must advance so the next interval is clean.
    const { dur, armedAt } = nextStallSample(1000, 60_000, false, 500, 50);
    assert.equal(dur, null);
    assert.equal(armedAt, 60_000);
  });

  it("cannot manufacture a phantom stall on the interval after a return", () => {
    // Background interval [1000, 60000] re-armed to 60000 with no record.
    const background = nextStallSample(1000, 60_000, false, 500, 50);
    // First foreground interval fires on time from the re-armed baseline.
    const resumed = nextStallSample(background.armedAt, 60_500, true, 500, 50);
    assert.equal(resumed.dur, null);
  });
});

describe("classifyInput", () => {
  it("splits felt latency into paint delay and pre-dispatch queue", () => {
    // event@100, received@250 (queued 150), painted@300 (latency 50) → felt 200.
    const result = classifyInput(100, 250, 300);
    assert.deepEqual(result, { latency: 50, queued: 150 });
  });

  it("drops the queue figure when the clock basis is implausible", () => {
    // event.timeStamp on a different origin: received - timeStamp is negative.
    const result = classifyInput(9_999_999, 250, 400);
    assert.deepEqual(result, { latency: 150, queued: null });
  });

  it("returns null when total felt latency is under threshold", () => {
    // queued 10 + latency 20 = 30ms felt, below the 100ms floor.
    assert.equal(classifyInput(100, 110, 130), null);
  });

  it("uses only paint latency toward the threshold when queue is implausible", () => {
    // queued dropped (negative); latency 120 alone clears the floor.
    const result = classifyInput(9_999_999, 250, 370);
    assert.deepEqual(result, { latency: 120, queued: null });
  });
});

describe("wrapInvoke / observer registry", () => {
  it("passes through untouched when no observer is registered", async () => {
    setInvokeObserver(null);
    const wrapped = wrapInvoke(
      (cmd) => Promise.resolve(`ok:${cmd}`),
      () => getInvokeObserver(),
    );
    assert.equal(await wrapped("ping"), "ok:ping");
  });

  it("records a resolved invoke once through the registered observer", async () => {
    const calls = [];
    const pending = new Map();
    let t = 0;
    setInvokeObserver(
      createInvokeObserver(
        pending,
        (cmd, dur, ok) => calls.push({ cmd, dur, ok }),
        () => (t += 5),
      ),
    );
    const wrapped = wrapInvoke(
      (cmd) => Promise.resolve(`ok:${cmd}`),
      () => getInvokeObserver(),
    );
    const result = await wrapped("ping");
    assert.equal(result, "ok:ping");
    assert.deepEqual(calls, [{ cmd: "ping", dur: 5, ok: true }]);
    assert.equal(pending.size, 0);
    setInvokeObserver(null);
  });

  it("records a rejected invoke and re-throws without leaking pending", async () => {
    const calls = [];
    const pending = new Map();
    setInvokeObserver(
      createInvokeObserver(pending, (cmd, _dur, ok) => calls.push({ cmd, ok })),
    );
    const wrapped = wrapInvoke(
      () => Promise.reject(new Error("boom")),
      () => getInvokeObserver(),
    );
    await assert.rejects(() => wrapped("bad"), /boom/);
    assert.deepEqual(calls, [{ cmd: "bad", ok: false }]);
    assert.equal(pending.size, 0);
    setInvokeObserver(null);
  });

  it("starts recording only after the observer is registered", async () => {
    setInvokeObserver(null);
    const calls = [];
    const wrapped = wrapInvoke(
      (cmd) => Promise.resolve(cmd),
      () => getInvokeObserver(),
    );
    // Module proxy wraps at load, before the harness starts: this call is a
    // transparent pass-through and must not be recorded.
    await wrapped("before");
    setInvokeObserver(
      createInvokeObserver(new Map(), (cmd) => calls.push(cmd)),
    );
    await wrapped("after");
    assert.deepEqual(calls, ["after"]);
    setInvokeObserver(null);
  });

  it("never touches a native non-configurable invoke property", async () => {
    // Native Tauri 2.11.5 defines `window.__TAURI_INTERNALS__.invoke` as a
    // non-configurable, non-writable value property; any redefine/assign
    // throws and would abort the harness. The module seam only reads the value
    // (as the real core module does per call) and wraps that function
    // reference — it must never write the property back.
    const internals = {};
    const native = (cmd) => Promise.resolve(`native:${cmd}`);
    Object.defineProperty(internals, "invoke", {
      configurable: false,
      writable: false,
      enumerable: true,
      value: native,
    });

    const calls = [];
    setInvokeObserver(
      createInvokeObserver(new Map(), (cmd, _dur, ok) =>
        calls.push({ cmd, ok }),
      ),
    );
    // The real core module derefs the property per call; the wrapper closes
    // over the function value, exactly like tauriCoreProxy.ts.
    const wrapped = wrapInvoke(internals.invoke, () => getInvokeObserver());
    const result = await wrapped("go");

    assert.equal(result, "native:go");
    assert.deepEqual(calls, [{ cmd: "go", ok: true }]);
    // The forbidden property is untouched and still non-configurable.
    const descriptor = Object.getOwnPropertyDescriptor(internals, "invoke");
    assert.equal(descriptor.configurable, false);
    assert.equal(descriptor.value, native);
    setInvokeObserver(null);
  });
});

describe("resolveTauriCore", () => {
  const PROXY = "/abs/src/shared/profiling/tauriCoreProxy.ts";
  const REAL = "/abs/node_modules/@tauri-apps/api/core.js";

  it("redirects the bare core specifier to the proxy", () => {
    // App code and external plugins (`plugin-opener`, …) import this form.
    assert.equal(
      resolveTauriCore("@tauri-apps/api/core", "/abs/src/App.tsx", PROXY, REAL),
      PROXY,
    );
  });

  it("redirects a relative ./core.js from inside @tauri-apps/api to the proxy", () => {
    // The bypass Thufir found: Tauri's own submodules (event/app/window/webview)
    // reach core via the relative import, which a bare alias never sees.
    assert.equal(
      resolveTauriCore(
        "./core.js",
        "/abs/node_modules/@tauri-apps/api/event.js",
        PROXY,
        REAL,
      ),
      PROXY,
    );
  });

  it("maps the proxy's escape specifier to the real core, not itself", () => {
    // The proxy re-exports through @tauri-core-impl; it must reach the real
    // module so the redirect never loops.
    assert.equal(
      resolveTauriCore("@tauri-core-impl", PROXY, PROXY, REAL),
      REAL,
    );
  });

  it("leaves the proxy's other imports untouched", () => {
    // e.g. the proxy importing the observer registry must not be redirected.
    assert.equal(
      resolveTauriCore("@/shared/profiling/ipc", PROXY, PROXY, REAL),
      null,
    );
  });

  it("ignores a relative ./core.js from outside @tauri-apps/api", () => {
    // A same-named file elsewhere must not be captured.
    assert.equal(
      resolveTauriCore(
        "./core.js",
        "/abs/src/shared/lib/thing.ts",
        PROXY,
        REAL,
      ),
      null,
    );
  });

  it("ignores unrelated specifiers", () => {
    assert.equal(
      resolveTauriCore("react", "/abs/src/App.tsx", PROXY, REAL),
      null,
    );
  });
});

describe("ProfileRecorder ring buffer", () => {
  it("bounds the buffer at RING_CAPACITY, dropping oldest", async () => {
    const rec = new ProfileRecorder("session-1", async () => {}, fixedClock());
    for (let i = 0; i < RING_CAPACITY + 50; i++) {
      rec.record({ type: "stall", dur: i });
    }
    assert.equal(rec.size(), RING_CAPACITY);
  });

  it("flushes serialized JSONL lines and drains the buffer", async () => {
    const captured = [];
    const rec = new ProfileRecorder(
      "session-1",
      async (lines) => {
        captured.push(...lines);
      },
      fixedClock(),
    );
    rec.record({ type: "stall", dur: 42 });
    rec.record({
      type: "census",
      observerEvents: 5,
      observerAgents: 1,
      observerMaxPerAgent: 5,
      archiveEvents: 0,
      transcripts: 1,
      queryCache: 3,
      domNodes: 100,
    });
    await rec.flush();

    assert.equal(rec.size(), 0);
    assert.equal(captured.length, 2);
    const first = JSON.parse(captured[0]);
    assert.equal(first.type, "stall");
    assert.equal(first.dur, 42);
    assert.equal(first.sid, "session-1");
    assert.equal(typeof first.t, "number");
    assert.equal(typeof first.wall, "number");
    assert.equal(typeof first.up, "number");
  });

  it("stamps the drop count onto the first line after overflow", async () => {
    const captured = [];
    const rec = new ProfileRecorder(
      "session-1",
      async (lines) => {
        captured.push(...lines);
      },
      fixedClock(),
    );
    for (let i = 0; i < RING_CAPACITY + 10; i++) {
      rec.record({ type: "stall", dur: i });
    }
    await rec.flush();
    const first = JSON.parse(captured[0]);
    assert.equal(first.dropped, 10);
  });

  it("does not throw when the sink rejects, and still drains", async () => {
    const rec = new ProfileRecorder(
      "session-1",
      async () => {
        throw new Error("sink down");
      },
      fixedClock(),
    );
    rec.record({ type: "stall", dur: 1 });
    await rec.flush();
    assert.equal(rec.size(), 0);
  });
});
