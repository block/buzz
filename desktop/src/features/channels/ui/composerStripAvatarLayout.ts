import * as React from "react";

/**
 * Per-strip namespace for the composer strip's avatar shared-element ids.
 *
 * The same agent avatar has two possible homes inside a strip — the merged
 * typing group and a status pill — and a partition flip (typing agent
 * promoted to a pill) should MORPH the avatar between them via a matching
 * motion `layoutId` instead of exiting one and entering the other.
 *
 * layoutIds are app-global, and the channel composer strip and the thread
 * panel strip can be mounted at once, so the ids must be namespaced per
 * strip instance — two simultaneously mounted elements sharing a layoutId
 * would fight over the "lead" and a morph could travel between strips.
 * Motion's own namespacing tool (LayoutGroup id) is deliberately NOT used:
 * LayoutGroup re-measures every layout-animating member whenever any member
 * updates, which turns each pill label ticker resize into a group-wide
 * onLayoutAnimationStart storm that permanently holds the pills' deferred
 * label swaps. A per-strip scope string baked into the id gives the
 * isolation without the group re-measurement semantics.
 *
 * BotActivityComposerAction provides the scope (its React.useId());
 * the pill and TypingIndicatorRow read it through
 * useComposerStripAvatarLayoutId. Outside a strip the context is null and
 * avatars render without a layoutId.
 */
export const ComposerStripAvatarScopeContext = React.createContext<
  string | null
>(null);

/**
 * Shared-element layout id for an agent's avatar within one strip scope.
 * Exposed for tests; components should prefer the hook below.
 */
export function composerStripAvatarLayoutId(scope: string, pubkey: string) {
  return `bot-activity-avatar-${scope}-${pubkey.toLowerCase()}`;
}

/**
 * The avatar's shared-element layout id in the enclosing composer strip, or
 * undefined when rendered outside a strip (no morph counterpart exists).
 */
export function useComposerStripAvatarLayoutId(
  pubkey: string,
): string | undefined {
  const scope = React.useContext(ComposerStripAvatarScopeContext);
  return scope === null
    ? undefined
    : composerStripAvatarLayoutId(scope, pubkey);
}

/**
 * Layout spring for the avatar morph, shared by both morph endpoints
 * (whichever end MOUNTS drives the transition). Quicker than the strip's
 * slot reorder spring — the morph travels the short hop between the
 * trailing typing group and the adjacent new pill and should read as one
 * quick slide.
 */
export const COMPOSER_STRIP_AVATAR_MORPH_TRANSITION = {
  layout: { type: "spring", duration: 0.45, bounce: 0.2 },
} as const;
