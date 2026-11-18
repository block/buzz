import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * The tenant scope whose UI is currently on screen, mirrored at module level
 * so code running outside React — the detached agent wake's `.catch` — can
 * ask "does my captured scope still describe what the user is looking at?".
 *
 * `<Toaster />` mounts outside the community remount boundary, so a toast
 * fired by a promise that outlived a community switch renders over the *new*
 * community's UI. The wake itself already fails closed at the backend; this
 * mirror fences only warning delivery, so community A's agent name and error
 * detail never surface while community B is on screen.
 *
 * Set by `useCommunityInit` when a community apply completes; cleared in
 * `resetCommunityState()` like every community-scoped module singleton. The
 * mirror is compared, not counted: an A→B→A round-trip restores A's scope, so
 * a slow start fired in A may still warn once the user is back in A — which
 * is exactly where "mention the agent again" is actionable. A reset
 * generation could not distinguish that from a one-way switch.
 */
type DetachedToastScope = {
  relayUrl: string;
  /** Null when the apply completed without a resolved identity. */
  signerPubkey: string | null;
};

let activeScope: DetachedToastScope | null = null;

/** Records the scope the just-applied community renders under. */
export function setDetachedToastScope(scope: DetachedToastScope): void {
  activeScope = scope;
}

/**
 * Clears the mirror. Registered in `resetCommunityState`: between a switch
 * and the next apply there is no on-screen scope, and delivery fails closed.
 */
export function resetDetachedToastScope(): void {
  activeScope = null;
}

/**
 * Whether a scope captured at fire time still matches the on-screen one.
 * Same comparison semantics as the backend's scope assertion: the relay URL
 * verbatim past a trim (its check is case-sensitive), the signer
 * case-insensitively. No mirror — mid-switch, or before the first apply —
 * means no match.
 */
export function matchesDetachedToastScope(
  relayUrl: string,
  signerPubkey: string,
): boolean {
  if (activeScope === null || activeScope.signerPubkey === null) {
    return false;
  }
  return (
    activeScope.relayUrl.trim() === relayUrl.trim() &&
    normalizePubkey(activeScope.signerPubkey) === normalizePubkey(signerPubkey)
  );
}
