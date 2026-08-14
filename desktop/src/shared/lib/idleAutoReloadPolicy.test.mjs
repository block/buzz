import assert from "node:assert/strict";
import test from "node:test";

import {
  createSerializedIdleReloadCheck,
  IDLE_RELOAD_OS_IDLE_MS,
  IDLE_RELOAD_SESSION_AGE_MS,
  normalizeOsIdleMs,
  runIdleReloadCheck,
  shouldIdleReload,
} from "./idleAutoReloadPolicy.ts";

// ── shouldIdleReload gate matrix ─────────────────────────────────────────────
//
// SAFETY-CRITICAL: a wrong "true" navigates the webview out from under the
// user. The all-green baseline is the ONLY fire case; each never-fire row
// flips exactly one input away from green to prove that gate alone holds.

/** All gates open, session old, user away — the only true case. */
function greenInputs(overrides = {}) {
  return {
    osIdleMs: IDLE_RELOAD_OS_IDLE_MS,
    sessionAgeMs: IDLE_RELOAD_SESSION_AGE_MS,
    appFocused: false,
    volatileWorkPending: false,
    ...overrides,
  };
}

test("shouldIdleReload_all_gates_green_returns_true", () => {
  assert.equal(shouldIdleReload(greenInputs()), true);
});

const NEVER_FIRE_ROWS = [
  ["volatile work pending blocks", { volatileWorkPending: true }],
  ["focused app blocks", { appFocused: true }],
  ["young session blocks", { sessionAgeMs: IDLE_RELOAD_SESSION_AGE_MS - 1 }],
  ["null idle (unsupported platform) blocks", { osIdleMs: null }],
  ["below idle threshold blocks", { osIdleMs: IDLE_RELOAD_OS_IDLE_MS - 1 }],
];

for (const [name, override] of NEVER_FIRE_ROWS) {
  test(`shouldIdleReload_${name.replace(/\s+/g, "_")}_returns_false`, () => {
    assert.equal(shouldIdleReload(greenInputs(override)), false);
  });
}

test("shouldIdleReload_exact_thresholds_are_inclusive", () => {
  assert.equal(
    shouldIdleReload(
      greenInputs({
        osIdleMs: IDLE_RELOAD_OS_IDLE_MS,
        sessionAgeMs: IDLE_RELOAD_SESSION_AGE_MS,
      }),
    ),
    true,
  );
});

test("shouldIdleReload_volatile_work_beats_all_other_green_gates", () => {
  // Every precondition green but volatile work outstanding → still false.
  assert.equal(
    shouldIdleReload(greenInputs({ volatileWorkPending: true })),
    false,
  );
});

// ── runIdleReloadCheck orchestration ─────────────────────────────────────────

const HOUR = 60 * 60 * 1000;

/** Readers wired to fire: old session, away, all clear. */
function greenReaders(overrides = {}) {
  const calls = { reload: 0 };
  const base = {
    now: () => 100 * HOUR,
    sessionLoadedAtMs: 100 * HOUR - IDLE_RELOAD_SESSION_AGE_MS,
    isAppFocused: () => false,
    getOsIdleMs: async () => IDLE_RELOAD_OS_IDLE_MS,
    getHuddleActive: async () => false,
    isVolatileWorkPending: () => false,
    reload: async () => {
      calls.reload += 1;
    },
  };
  return { readers: { ...base, ...overrides }, calls };
}

test("runIdleReloadCheck_fires_reload_when_all_conditions_hold", async () => {
  const { readers, calls } = greenReaders();
  assert.equal(await runIdleReloadCheck(readers), true);
  assert.equal(calls.reload, 1);
});

test("runIdleReloadCheck_young_session_skips_before_any_ipc", async () => {
  let osIdleReads = 0;
  let huddleReads = 0;
  const { readers, calls } = greenReaders({
    sessionLoadedAtMs: 100 * HOUR - (IDLE_RELOAD_SESSION_AGE_MS - 1),
    getOsIdleMs: async () => {
      osIdleReads += 1;
      return IDLE_RELOAD_OS_IDLE_MS;
    },
    getHuddleActive: async () => {
      huddleReads += 1;
      return false;
    },
  });
  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0);
  // A present-enough session must not incur the per-minute IPC reads.
  assert.equal(osIdleReads, 0);
  assert.equal(huddleReads, 0);
});

test("runIdleReloadCheck_focused_user_skips_before_any_ipc", async () => {
  let osIdleReads = 0;
  const { readers, calls } = greenReaders({
    isAppFocused: () => true,
    getOsIdleMs: async () => {
      osIdleReads += 1;
      return IDLE_RELOAD_OS_IDLE_MS;
    },
  });
  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0);
  assert.equal(osIdleReads, 0);
});

test("runIdleReloadCheck_huddle_active_blocks_reload", async () => {
  const { readers, calls } = greenReaders({
    getHuddleActive: async () => true,
  });
  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0);
});

test("runIdleReloadCheck_volatile_work_blocks_reload", async () => {
  // Stands in for a foreground/background upload, queued local attachment, or
  // in-flight send/mutation/native operation — the hook folds all into one read.
  const { readers, calls } = greenReaders({
    isVolatileWorkPending: () => true,
  });
  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0);
});

test("runIdleReloadCheck_null_os_idle_blocks_reload", async () => {
  const { readers, calls } = greenReaders({
    getOsIdleMs: async () => null,
  });
  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0);
});

// ── normalizeOsIdleMs — the extracted reader normalizer ──────────────────────
//
// The MINOR finding: prove a rejecting getOsIdleSeconds withholds the reload
// this tick, then a later successful read lets it fire. The normalizer is the
// seam that collapses all three "cannot confirm away" outcomes to null; it was
// extracted to the pure module precisely so this path tests without the hook's
// React/timer/IPC weight.

test("normalizeOsIdleMs_seconds_convert_to_milliseconds", async () => {
  assert.equal(await normalizeOsIdleMs(async () => 42), 42_000);
});

test("normalizeOsIdleMs_null_reader_stays_null", async () => {
  assert.equal(await normalizeOsIdleMs(async () => null), null);
});

test("normalizeOsIdleMs_rejecting_reader_collapses_to_null", async () => {
  // A failed idle IPC must never surface as a numeric idle the policy could
  // read as "away" — it collapses to null, which shouldIdleReload withholds on.
  assert.equal(
    await normalizeOsIdleMs(async () => {
      throw new Error("idle IPC failed");
    }),
    null,
  );
});

test("idle_read_rejection_withholds_this_tick_then_retry_fires", async () => {
  // The full MINOR contract through the real normalizer + orchestrator: the
  // first tick's idle read rejects → normalizeOsIdleMs → null → withheld; the
  // next tick's read succeeds → the reload fires. No stuck state between ticks.
  let attempt = 0;
  const getOsIdleSeconds = async () => {
    attempt += 1;
    if (attempt === 1) throw new Error("transient idle IPC failure");
    return IDLE_RELOAD_OS_IDLE_MS / 1000;
  };
  const { readers, calls } = greenReaders({
    getOsIdleMs: () => normalizeOsIdleMs(getOsIdleSeconds),
  });

  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0, "a rejected idle read must withhold this tick");

  assert.equal(await runIdleReloadCheck(readers), true);
  assert.equal(calls.reload, 1, "the next tick's successful read must fire");
});

test("runIdleReloadCheck_user_returns_during_ipc_await_blocks_reload", async () => {
  // Focus flips to true while the async idle/huddle reads are in flight; the
  // post-await re-read of the synchronous focus signal must catch it.
  let focused = false;
  const { readers, calls } = greenReaders({
    isAppFocused: () => focused,
    getOsIdleMs: async () => {
      focused = true; // user touched the machine mid-check
      return IDLE_RELOAD_OS_IDLE_MS;
    },
  });
  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0);
});

test("runIdleReloadCheck_volatile_work_appears_during_ipc_await_blocks_reload", async () => {
  let busy = false;
  const { readers, calls } = greenReaders({
    isVolatileWorkPending: () => busy,
    getHuddleActive: async () => {
      busy = true; // a send/upload began mid-check
      return false;
    },
  });
  assert.equal(await runIdleReloadCheck(readers), false);
  assert.equal(calls.reload, 0);
});

// ── createSerializedIdleReloadCheck ──────────────────────────────────────────

test("serialized_check_drops_ticks_while_a_check_is_pending", async () => {
  let releaseIdle;
  let osIdleReads = 0;
  const { readers, calls } = greenReaders({
    getOsIdleMs: () => {
      osIdleReads += 1;
      return new Promise((resolve) => {
        releaseIdle = () => resolve(IDLE_RELOAD_OS_IDLE_MS);
      });
    },
  });
  const check = createSerializedIdleReloadCheck(readers);
  check(); // starts a check that parks on the idle read
  check(); // second tick lands while the first is pending — dropped
  check();
  assert.equal(osIdleReads, 1);
  releaseIdle();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls.reload, 1);
});

test("serialized_check_runs_again_after_the_previous_check_settles", async () => {
  let osIdleReads = 0;
  const { readers, calls } = greenReaders({
    getOsIdleMs: async () => {
      osIdleReads += 1;
      return IDLE_RELOAD_OS_IDLE_MS;
    },
    reload: async () => {
      calls.reload += 1;
    },
  });
  const check = createSerializedIdleReloadCheck(readers);
  check();
  await new Promise((resolve) => setTimeout(resolve, 0));
  check();
  await new Promise((resolve) => setTimeout(resolve, 0));
  // Both ticks ran to completion because each settled before the next.
  assert.equal(osIdleReads, 2);
});
