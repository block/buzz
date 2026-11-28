// Per-channel lifecycle for the unread catch-up fetch:
//
//   (absent) --claim--> inFlight --complete--> done
//                          |
//                          +------release-----> (absent)   (error / scope not ready)
//
// This used to be a bare Set<string> plus an `isCancelled`/`isCompleted`
// pair of closure booleans in the effect. "Claimed" could not tell "request in
// flight" from "result merged", so effect cleanup (fired on every getter
// identity change) released in-flight claims, dropped the pending result and
// re-issued the same 1000-events-per-channel fetch: the catch-up replay loop
// behind the observed-unread.db hot loop. Making the state explicit lets
// cleanup leave in-flight work alone.
//
// Identity reset: the owner swaps in a NEW instance (see useUnreadChannels), and
// a result handler compares its captured instance to the current one to drop
// stale-scope results. No epoch counter is needed.

type ClaimState = "inFlight" | "done";

export class CatchUpClaims {
  private readonly states = new Map<string, ClaimState>();

  /** Claims every unclaimed id and returns exactly those ids (the ones to fetch). */
  claim(ids: readonly string[]): string[] {
    const claimed: string[] = [];
    for (const id of ids) {
      if (this.states.has(id)) continue;
      this.states.set(id, "inFlight");
      claimed.push(id);
    }
    return claimed;
  }

  /** inFlight -> done. Only legal from inFlight; anything else is ignored. */
  complete(id: string): void {
    if (this.states.get(id) === "inFlight") this.states.set(id, "done");
  }

  /** inFlight -> absent so a later run retries. A `done` channel is never un-done. */
  release(id: string): void {
    if (this.states.get(id) === "inFlight") this.states.delete(id);
  }
}
