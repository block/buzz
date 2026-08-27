/**
 * Single-slot coordinator for cover drawer focus restoration.
 *
 * There is one covered slot in a channel, so there is one focus claim. A drawer
 * takes a claim when it captures focus and checks it back at teardown: it hands
 * focus to whatever it stole it from only if its claim is still the current one.
 *
 * This exists because restoration is deferred a frame (the drawer has to let the
 * exit animation start before moving focus), and a lot can happen in that frame.
 * When one drawer replaces another the successor mounts and focuses itself while
 * the outgoing one is still animating out, so an unconditional restore would
 * yank focus out of the new drawer and into the channel that is now inert —
 * unreachable by keyboard, with no visible focus ring anywhere.
 *
 * A monotonic generation answers "was I superseded?" without anyone having to
 * name their successor or reason about mount ordering: any newer claim, from any
 * source, invalidates every older one. That keeps the decision out of the drawer
 * primitive, which cannot see the surrounding presentation and should not be
 * interpreting it.
 */

let generation = 0;

/**
 * Take the focus slot for a drawer that has just captured focus.
 *
 * The returned claim is opaque; pass it to {@link hasCoverDrawerFocusClaim} at
 * teardown to find out whether this drawer is still the one that owes focus back.
 */
export function claimCoverDrawerFocus(): number {
  generation += 1;
  return generation;
}

/** Whether `claim` is still the current claim, i.e. nothing has superseded it. */
export function hasCoverDrawerFocusClaim(claim: number): boolean {
  return claim === generation;
}

/**
 * Invalidate the outstanding claim because focus has been placed deliberately
 * elsewhere.
 *
 * For transitions that retire a drawer without another drawer replacing it, and
 * that have already decided where focus belongs — switching a thread from the
 * focus drawer to the split pane, which moves focus to the view-mode toggle or
 * the thread body itself. Without this the drawer's own restore would fire a
 * frame later and pull focus back to whatever opened the thread.
 */
export function releaseCoverDrawerFocus(): void {
  generation += 1;
}
