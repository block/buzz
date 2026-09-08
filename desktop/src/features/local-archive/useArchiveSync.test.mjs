/**
 * Mounted-hook tests for the archive sync start gate.
 *
 * The gate is the only part of archive sync still living in the renderer, and
 * it is load-bearing for a reason no unit test of either hook alone can show:
 * kind 24200 is relay-*ephemeral*. Frames that arrive before the listener
 * opens are gone permanently, so `start_archive_sync` must not be invoked
 * until observer reconciliation has seeded kind 24200 into the saved
 * subscription. Everything below the gate now runs in Rust and is covered by
 * `archive/sync_tests.rs`.
 *
 * These mount the two real hooks composed exactly as AppShell composes them
 * (`useArchiveSync(useObserverArchiveReconciliation(pubkey))`) and assert on
 * the Tauri commands that actually cross the boundary. They replace the
 * ordering tests that drove the deleted `ArchiveSyncManager`: the invariant
 * survived the move to Rust, only its observable did.
 */

import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";

import { JSDOM } from "jsdom";
import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { useObserverArchiveReconciliation } from "./useObserverArchiveSeed.ts";
import { useArchiveSync } from "./useArchiveSync.ts";

const PUBKEY = "pk1";

/**
 * Loads a fresh instance of the hook module — a new JS realm.
 *
 * The realm's epoch is memoized in module scope, which is exactly right in
 * production (one realm announces once) and exactly what a reload destroys.
 * Re-importing under a unique specifier reproduces that destruction honestly,
 * rather than exporting a reset hook that exists only for tests.
 */
async function freshRealm() {
  const mod = await import(`./useArchiveSync.ts?realm=${realmCounter++}`);
  return mod.useArchiveSync;
}
let realmCounter = 0;

// ── Harness ──────────────────────────────────────────────────────────────────

/**
 * Mounts the AppShell composition and returns the invoked Tauri command names
 * in call order, plus handles to drive the gate and unmount.
 *
 * `@tauri-apps/api/core` reads `window.__TAURI_INTERNALS__.invoke` at call
 * time, so intercepting it here captures every command the hooks issue.
 */
function mountGate({
  mergeShouldFail = false,
  windowLabel = "main",
  useArchiveSync: useArchiveSyncImpl = useArchiveSync,
} = {}) {
  const dom = new JSDOM(
    "<!doctype html><html><body><div id='root'></div></body></html>",
  );
  const invoked = [];
  const calls = [];
  let epochs = 0;
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      invoked.push(cmd);
      calls.push({ cmd, epoch: args?.epoch, lease: args?.lease });
      if (cmd === "announce_archive_sync_epoch") {
        epochs += 1;
        return Promise.resolve(epochs);
      }
      return Promise.resolve(null);
    },
    // `getCurrentWindow()` reads exactly this path (api/window.js:85), and
    // `isTauri()` reads `globalThis.isTauri` (api/core.js:280). Both are what
    // `huddleWindowChannelId` uses to tell a companion realm from the main one.
    metadata: { currentWindow: { label: windowLabel } },
    transformCallback: () => Math.random(),
  };
  Object.assign(globalThis, {
    isTauri: true,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });

  let resolveMerge;
  let rejectMerge;
  const mergePromise = new Promise((resolve, reject) => {
    resolveMerge = resolve;
    rejectMerge = () => reject(new Error("merge failed"));
  });

  // Real reconciler, controllable merge: this is the production seam the
  // reconciliation hook already exposes for tests. Frozen so the hook's
  // effect does not re-run on every render.
  const deps = Object.freeze({
    mergeSaveSubscriptionKinds: () => mergePromise,
    readExplicitChoice: () => "unset",
    setExplicitChoice: () => {},
  });

  function Harness() {
    const reconciled = useObserverArchiveReconciliation(PUBKEY, deps);
    useArchiveSyncImpl(reconciled);
    return null;
  }

  const root = createRoot(dom.window.document.getElementById("root"));

  return {
    invoked,
    calls,
    async mount() {
      await act(async () => {
        root.render(React.createElement(Harness));
      });
    },
    async settleGate() {
      await act(async () => {
        if (mergeShouldFail) rejectMerge();
        else resolveMerge();
        await mergePromise.catch(() => {});
      });
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

afterEach(() => {
  delete globalThis.isTauri;
  delete globalThis.document;
  delete globalThis.window;
  delete globalThis.HTMLElement;
  delete globalThis.IS_REACT_ACT_ENVIRONMENT;
});

// ── The gate ─────────────────────────────────────────────────────────────────

describe("archive sync start gate", () => {
  it("does not start the backend task before reconciliation resolves", async () => {
    const gate = mountGate();
    await gate.mount();

    assert.deepEqual(
      gate.invoked.filter((cmd) => cmd === "start_archive_sync"),
      [],
      "start_archive_sync must not run before reconciliation resolves — " +
        "kind 24200 frames arriving before the listener opens are lost",
    );

    await gate.settleGate();

    assert.deepEqual(
      gate.invoked.filter((cmd) => cmd === "start_archive_sync"),
      ["start_archive_sync"],
      "start_archive_sync must run exactly once after the gate opens",
    );
  });

  it("never starts the backend task when reconciliation fails", async () => {
    const gate = mountGate({ mergeShouldFail: true });
    await gate.mount();
    await gate.settleGate();

    assert.deepEqual(
      gate.invoked.filter((cmd) => cmd === "start_archive_sync"),
      [],
      "a failed reconciliation leaves the gate closed",
    );
  });

  it("stops the backend task on unmount", async () => {
    const gate = mountGate();
    await gate.mount();
    await gate.settleGate();
    await gate.unmount();

    assert.deepEqual(
      gate.invoked.filter((cmd) => cmd.endsWith("_archive_sync")),
      ["start_archive_sync", "stop_archive_sync"],
      "unmount must stop the task it started, in that order",
    );
  });
});

// ── Lifecycle ownership ──────────────────────────────────────────────────────
//
// Both commands are fired without awaiting, and Tauri completes them in an
// unconstrained order. The lease is what lets the backend recover the
// renderer's intent order from calls that arrive in any order, so what these
// assert is that the renderer allocates it correctly — synchronously, in effect
// order, with each cleanup carrying its own start's lease.
//
// The backend half of the invariant (a stale lease changes nothing) is proven
// in `archive/sync_tests.rs`, where the completion order can be chosen
// directly. Neither test can stand alone: this one cannot control backend
// ordering, and that one cannot show which lease the renderer sent.

describe("archive sync lifecycle leases", () => {
  it("pairs each stop with the lease of the start it is cleaning up", async () => {
    const gate = mountGate();
    await gate.mount();
    await gate.settleGate();
    await gate.unmount();

    const lifecycle = gate.calls.filter(({ cmd }) =>
      cmd.endsWith("_archive_sync"),
    );
    assert.equal(lifecycle.length, 2, "expected one start and one stop");

    const [start, stop] = lifecycle;
    assert.equal(typeof start.lease, "number", "start must carry a lease");
    assert.equal(
      stop.lease,
      start.lease,
      "cleanup must carry its own start's lease — a stop that sends anything " +
        "else either cancels a newer owner's task or no-ops its own",
    );
  });

  it("allocates a strictly newer lease for each remount", async () => {
    const first = mountGate();
    await first.mount();
    await first.settleGate();
    await first.unmount();

    const second = mountGate();
    await second.mount();
    await second.settleGate();

    const leaseOf = (gate, cmd) =>
      gate.calls.find((call) => call.cmd === cmd)?.lease;

    assert.ok(
      leaseOf(second, "start_archive_sync") >
        leaseOf(first, "start_archive_sync"),
      "a remount's start must outrank the previous mount's, or the backend " +
        "cannot tell which start the app actually wants",
    );
    assert.ok(
      leaseOf(second, "start_archive_sync") >
        leaseOf(first, "stop_archive_sync"),
      "a remount's start must also outrank the previous cleanup, so a stop " +
        "that lands late cannot strand the new task",
    );
  });
});

// ── Realm ownership ──────────────────────────────────────────────────────────
//
// `AppShell` is the root route, so a companion huddle window mounts this same
// tree in a second JS realm. Archive sync is app-global work and the main
// window owns it, exactly as it owns microphone capture. The epoch cannot fix
// this: epochs order realms in TIME, and a companion is a second realm in
// SPACE — its unmount cleanup would carry the newest epoch and cancel the live
// main-window task. So the companion must never announce or issue lifecycle
// commands at all.

describe("archive sync realm ownership", () => {
  it("issues no archive lifecycle commands from a companion window", async () => {
    const gate = mountGate({
      windowLabel: "huddle-11111111-2222-3333-4444-555555555555",
    });
    await gate.mount();
    await gate.settleGate();
    await gate.unmount();

    assert.deepEqual(
      gate.invoked.filter(
        (cmd) =>
          cmd.endsWith("_archive_sync") ||
          cmd === "announce_archive_sync_epoch",
      ),
      [],
      "a companion realm must not announce or run lifecycle commands — its " +
        "cleanup would cancel the main window's live sync task",
    );
  });

  it("still runs the lifecycle in the main window", async () => {
    const gate = mountGate({ windowLabel: "main" });
    await gate.mount();
    await gate.settleGate();
    await gate.unmount();

    assert.deepEqual(
      gate.invoked.filter((cmd) => cmd.endsWith("_archive_sync")),
      ["start_archive_sync", "stop_archive_sync"],
      "the owning window must be unaffected by the companion exclusion",
    );
  });

  it("resolves the epoch before issuing any lifecycle command", async () => {
    const gate = mountGate({ useArchiveSync: await freshRealm() });
    await gate.mount();
    await gate.settleGate();
    await gate.unmount();

    const relevant = gate.invoked.filter(
      (cmd) =>
        cmd.endsWith("_archive_sync") || cmd === "announce_archive_sync_epoch",
    );
    assert.equal(
      relevant[0],
      "announce_archive_sync_epoch",
      "the epoch must be announced before any lifecycle command — an " +
        "unawaited announcement is just another racing invoke",
    );

    const lifecycle = gate.calls.filter(({ cmd }) =>
      cmd.endsWith("_archive_sync"),
    );
    for (const call of lifecycle) {
      assert.equal(
        typeof call.epoch,
        "number",
        `${call.cmd} must carry the resolved epoch`,
      );
    }
    assert.equal(
      lifecycle[0].epoch,
      lifecycle[1].epoch,
      "start and its cleanup must share one realm epoch",
    );
  });
});
