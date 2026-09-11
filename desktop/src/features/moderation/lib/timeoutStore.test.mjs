/**
 * The moderation timeout store.
 *
 * A timeout is issued by one relay against one community membership. The store
 * holding it is a module-level singleton, so it survives the React remount that
 * a community switch performs — which is why `resetCommunityState` has to clear
 * it explicitly. Without that, a timeout taken in community A disables the
 * composer in community B (`ChannelPane` gates `isComposerDisabled` on
 * `timeoutState.active`).
 *
 * The unknown-expiry case is the sharp one: `isTimeoutActive(null)` is `true`
 * forever, and the only other thing that clears the store is an accepted send —
 * which the disabled composer will not let the member make.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  clearTimeoutState,
  getTimeoutSnapshot,
  recordTimeoutFromRejection,
} from "./timeoutStore.ts";

describe("moderation timeout store", () => {
  beforeEach(() => {
    clearTimeoutState();
  });

  it("records a timeout with a known expiry", () => {
    const expiresAt = Date.now() + 60_000;
    assert.equal(
      recordTimeoutFromRejection(
        `restricted: you are timed out until ${Math.floor(expiresAt / 1000)}`,
      ),
      true,
    );
    const snapshot = getTimeoutSnapshot();
    assert.equal(snapshot.active, true);
    assert.equal(typeof snapshot.expiresAtMs, "number");
  });

  it("leaves an unrelated rejection untouched", () => {
    assert.equal(recordTimeoutFromRejection("rate limited: slow down"), false);
    assert.equal(getTimeoutSnapshot().active, false);
    assert.equal(recordTimeoutFromRejection(null), false);
    assert.equal(recordTimeoutFromRejection(undefined), false);
  });

  it("clearing restores the inactive snapshot", () => {
    recordTimeoutFromRejection(
      "restricted: you are timed out until 99999999999",
    );
    assert.equal(getTimeoutSnapshot().active, true);

    clearTimeoutState();
    assert.deepEqual(getTimeoutSnapshot(), {
      active: false,
      expiresAtMs: null,
    });
  });

  it("clearing releases a timeout that carries no expiry", () => {
    // `isTimeoutActive(null)` is true forever, so nothing lapses this one on its
    // own. If a community switch does not clear it, the member is write-blocked
    // everywhere with no way out.
    // The relay named the block but gave no parseable expiry -- the shape
    // `parseTimeoutRejection` maps to `{ expiresAtMs: null }`.
    recordTimeoutFromRejection("restricted: you are timed out until 0");
    const recorded = getTimeoutSnapshot();
    assert.equal(recorded.active, true);
    assert.equal(recorded.expiresAtMs, null);

    clearTimeoutState();
    assert.equal(getTimeoutSnapshot().active, false);
  });

  it("clearing an already-clear store is a no-op", () => {
    const before = getTimeoutSnapshot();
    clearTimeoutState();
    assert.equal(
      getTimeoutSnapshot(),
      before,
      "snapshot identity must stay stable so useSyncExternalStore does not loop",
    );
  });

  it("a later rejection replaces the recorded expiry", () => {
    recordTimeoutFromRejection(
      "restricted: you are timed out until 1000000000",
    );
    const first = getTimeoutSnapshot().expiresAtMs;
    recordTimeoutFromRejection(
      "restricted: you are timed out until 2000000000",
    );
    const second = getTimeoutSnapshot().expiresAtMs;
    assert.notEqual(first, second);
    assert.equal(getTimeoutSnapshot().active, true);
  });
});
