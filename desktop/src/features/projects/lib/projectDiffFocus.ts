import type { DiffLineAnnotation, SelectedLineRange } from "@pierre/diffs";
import { getLineAnnotationName } from "@pierre/diffs";

import type { ProjectPullRequestCommentAnchor } from "@/features/projects/projectPullRequests.mjs";
import {
  buzzSideToDiffSide,
  focusedAnchorToSelectedRange,
  type ProjectDiffAnnotationMetadata,
  type ProjectDiffSide,
} from "@/features/projects/lib/projectDiffAnnotations";

/**
 * Isolated Shadow DOM lookup for the focused-line scroll/focus path.
 *
 * Pierre renders into a `diffs-container` element with an open shadow root.
 * Every line annotation we pass through `lineAnnotations` produces a stable
 * annotation slot named by the public `getLineAnnotationName` utility (e.g.
 * `annotation-additions-5`) inside a `[data-line-annotation]` row wrapper.
 * Because the adapter always includes the focused anchor as an annotation
 * (even with zero comments), this lookup has a stable, documented hook to
 * resolve — no broad selectors, no reliance on internal row ordering.
 */

/** The row element that owns the given annotation slot, or null. */
export function findAnnotationRow(
  container: Element,
  side: ProjectDiffSide,
  lineNumber: number,
): HTMLElement | null {
  const shadowRoot = container.shadowRoot;
  if (!shadowRoot) return null;
  const slotName = getLineAnnotationName({ side, lineNumber });
  const slot = shadowRoot.querySelector(`slot[name="${slotName}"]`);
  if (!slot) return null;
  return slot.closest("[data-line-annotation]");
}

/**
 * Focus the annotation row for the given diff side/line: move it into view,
 * make it programmatically focusable, and focus it without re-scrolling.
 * Returns false when the row cannot be resolved (e.g. the line does not
 * exist in the rendered patch).
 */
export function focusAnnotationRow(
  container: Element,
  side: ProjectDiffSide,
  lineNumber: number,
): boolean {
  const row = findAnnotationRow(container, side, lineNumber);
  if (!row) return false;
  row.scrollIntoView({ behavior: "smooth", block: "center" });
  row.setAttribute("tabindex", "-1");
  row.focus({ preventScroll: true });
  return true;
}

/** Stable ordering key for a diff annotation: deletions before additions. */
function sideOrder(side: ProjectDiffSide) {
  return side === "deletions" ? 0 : 1;
}

/** Stable per-file annotation key (path is already scoped by the caller). */
function annotationKey(
  annotation: DiffLineAnnotation<ProjectDiffAnnotationMetadata>,
) {
  return `${annotation.side}:${annotation.lineNumber}`;
}

/**
 * Guarantee an annotation slot for the panel's active comment anchor.
 *
 * `buildFileDiffAnnotations` covers existing comment groups and the focused
 * anchor, but a freshly opened composer lives on a line that may have no
 * comments and no focus. Without an annotation for `activeAnchor` Pierre
 * never calls `renderAnnotation` for it and the composer is invisible.
 *
 * Merges a commentless annotation for the active anchor into an existing
 * annotation list, deduplicating identical side/line anchors and re-applying
 * deterministic ordering (line ascending, deletions before additions). An
 * anchor from another file is ignored. When the active anchor is the same as
 * the focused anchor it is already present (focused anchors are always
 * included) and is left untouched.
 */
export function includeActiveDiffAnchor(
  annotations: DiffLineAnnotation<ProjectDiffAnnotationMetadata>[],
  path: string,
  activeAnchor: ProjectPullRequestCommentAnchor | null | undefined,
): DiffLineAnnotation<ProjectDiffAnnotationMetadata>[] {
  if (!activeAnchor || activeAnchor.path !== path) return annotations;

  const active: DiffLineAnnotation<ProjectDiffAnnotationMetadata> = {
    side: buzzSideToDiffSide(activeAnchor.side),
    lineNumber: activeAnchor.line,
    metadata: {
      anchor: activeAnchor,
      comments: [],
      focused: false,
    },
  };
  const existing = new Set(annotations.map(annotationKey));
  if (existing.has(annotationKey(active))) return annotations;

  return [...annotations, active].sort(
    (left, right) =>
      left.lineNumber - right.lineNumber ||
      sideOrder(left.side) - sideOrder(right.side),
  );
}

/**
 * The focused anchor's Pierre selected-line range, scoped to the file being
 * rendered. Returns null when there is no focused anchor or when it points at
 * a different path — a mismatched-path focus must not produce a transient or
 * wrong-line selection on the previously displayed file.
 */
export function selectedRangeForFile(
  path: string,
  focusedAnchor: ProjectPullRequestCommentAnchor | null | undefined,
): SelectedLineRange | null {
  if (!focusedAnchor || focusedAnchor.path !== path) return null;
  return focusedAnchorToSelectedRange(focusedAnchor);
}

/**
 * One-shot focus key for the focused anchor, scoped to the file being
 * rendered. Includes the path so switching between files at the same side and
 * line is treated as a new focus target. Returns null when there is no
 * focused anchor or when it points at a different path.
 */
export function focusedAnchorKey(
  path: string,
  focusedAnchor: ProjectPullRequestCommentAnchor | null | undefined,
): string | null {
  if (!focusedAnchor || focusedAnchor.path !== path) return null;
  return `${focusedAnchor.path}:${focusedAnchor.side}:${focusedAnchor.line}`;
}

/**
 * Mutable one-shot state for focused-line scroll/focus, kept in a ref by the
 * adapter and advanced through {@link nextFocusAttempt} /
 * {@link markFocusSucceeded} so the lifecycle is pure and testable.
 */
export type FocusOneShotState = {
  /** Last key that was successfully focused, or null when none/cleared. */
  lastFocusedKey: string | null;
};

/** Initial one-shot state: nothing focused yet. */
export function createFocusOneShotState(): FocusOneShotState {
  return { lastFocusedKey: null };
}

/**
 * Advance the one-shot focused-line lifecycle for one public `onPostRender`
 * pass. The file-scoped focus key is null when there is no focused anchor or
 * it points at another file.
 *
 * - Null key: clears the remembered key (focus was cleared or the file
 *   switched away), returns `attempt: false`. A later identical anchor is
 *   treated as a genuinely new focus target.
 * - Key equal to the remembered key: `attempt: false` — one-shot behavior
 *   after a successful focus.
 * - Any other key: `attempt: true`; the returned state is unchanged so the
 *   caller only records success via {@link markFocusSucceeded} after
 *   `focusAnnotationRow` actually resolves — a failed lookup stays retryable
 *   on a later post-render.
 */
export function nextFocusAttempt(
  state: FocusOneShotState,
  key: string | null,
): { state: FocusOneShotState; attempt: boolean } {
  if (key === null) {
    return { state: { lastFocusedKey: null }, attempt: false };
  }
  if (state.lastFocusedKey === key) {
    return { state, attempt: false };
  }
  return { state, attempt: true };
}

/** Record a successfully focused key so the same key is not re-focused. */
export function markFocusSucceeded(key: string): FocusOneShotState {
  return { lastFocusedKey: key };
}

/**
 * Exact hovered line cached from Pierre's public `onLineEnter` payload.
 * Scoped to one rendered file path so no cross-file anchor can survive a
 * file switch. The gutter action resolves live `getHoveredLine()` first and
 * falls back to this cache only when the live getter is absent.
 */
export type HoveredLineAnchor = {
  lineNumber: number;
  side: ProjectDiffSide;
};

/** The cached onLineEnter anchor, or null when none is known yet. */
export type HoverAnchorCache = {
  path: string;
  anchor: HoveredLineAnchor;
} | null;

/** Start with no cached hovered line. */
export function createHoverAnchorCache(): HoverAnchorCache {
  return null;
}

/**
 * Record the last exact `{ lineNumber, side }` delivered by Pierre's public
 * `onLineEnter` callback for the rendered file. A later `onLineEnter` for the
 * same file replaces it; switching files replaces the whole entry.
 */
export function recordHoveredLine(
  path: string,
  lineNumber: number,
  side: ProjectDiffSide,
): HoverAnchorCache {
  return { path, anchor: { lineNumber, side } };
}

/**
 * The cached hovered line for the current file, or null when the cache holds
 * a different file (a stale anchor must never survive a file switch).
 */
export function cachedHoveredLine(
  cache: HoverAnchorCache,
  path: string,
): HoveredLineAnchor | null {
  return cache && cache.path === path ? cache.anchor : null;
}

/**
 * Resolve the anchor for the gutter action: the live getter always wins; the
 * cached public `onLineEnter` anchor is the fallback; null when neither
 * exists. Never infers a line from DOM structure or selection internals.
 */
export function resolveGutterAnchor(
  cache: HoverAnchorCache,
  path: string,
  live: HoveredLineAnchor | null | undefined,
): HoveredLineAnchor | null {
  if (live) return live;
  return cachedHoveredLine(cache, path);
}
