import type { DiffLineAnnotation, SelectedLineRange } from "@pierre/diffs";

import type {
  ProjectPullRequestComment,
  ProjectPullRequestCommentAnchor,
} from "@/features/projects/projectPullRequests.mjs";

/**
 * Buzz-owned annotation metadata carried inside a Pierre
 * {@link DiffLineAnnotation}. It keeps the exact Buzz anchor and its comment
 * group so `renderAnnotation` can rebuild the existing inline thread/form
 * without re-deriving state from the parsed patch.
 */
export type ProjectDiffAnnotationMetadata = {
  /** The exact Buzz anchor this annotation was built from. */
  anchor: ProjectPullRequestCommentAnchor;
  /** Comments already grouped to this path/side/line, in stable order. */
  comments: ProjectPullRequestComment[];
  /** True when this annotation is the active focused line. */
  focused: boolean;
};

/** Pierre diff sides, mirroring {@link DiffLineAnnotation.side}. */
export type ProjectDiffSide = "deletions" | "additions";

/**
 * Map a Buzz anchor side to the Pierre diff side.
 * Buzz `old` anchors live on the deletion side; Buzz `new` anchors live on
 * the addition side.
 */
export function buzzSideToDiffSide(
  side: ProjectPullRequestCommentAnchor["side"],
): ProjectDiffSide {
  return side === "old" ? "deletions" : "additions";
}

/**
 * Map a Pierre diff side back to the Buzz anchor side.
 * Inverse of {@link buzzSideToDiffSide}.
 */
export function diffSideToBuzzSide(side: ProjectDiffSide): "old" | "new" {
  return side === "deletions" ? "old" : "new";
}

/** Stable per-file grouping key for a comment anchor. */
function anchorGroupKey(anchor: ProjectPullRequestCommentAnchor) {
  return `${anchor.side}:${anchor.line}`;
}

/**
 * Group a file's inline comments by path/side/line. The returned map order is
 * first-seen key order (input order determines group order); the stable,
 * sorted ordering callers need is produced by {@link buildFileDiffAnnotations}
 * when it renders the final annotation list. Within each group, the original
 * comment order is preserved. Comments whose anchor points at another file
 * are excluded.
 */
export function groupCommentsByAnchor(
  path: string,
  comments: ProjectPullRequestComment[],
): Map<string, ProjectPullRequestComment[]> {
  const groups = new Map<string, ProjectPullRequestComment[]>();
  for (const comment of comments) {
    const anchor = comment.anchor;
    if (!anchor || anchor.path !== path) continue;
    const key = anchorGroupKey(anchor);
    const group = groups.get(key);
    if (group) group.push(comment);
    else groups.set(key, [comment]);
  }
  return groups;
}

/**
 * Build the Pierre line annotations for one file from its comments and the
 * active focused anchor.
 *
 * Every comment group becomes one annotation carrying the exact Buzz anchor
 * and its comments. The focused anchor is always included, even when it has
 * no comments, so focused-line scroll/focus has a stable annotation slot to
 * resolve inside the rendered diff. Results are ordered deterministically by
 * (line, side) so annotation slots are stable across renders.
 */
export function buildFileDiffAnnotations(
  path: string,
  comments: ProjectPullRequestComment[],
  focusedAnchor: ProjectPullRequestCommentAnchor | null | undefined,
): DiffLineAnnotation<ProjectDiffAnnotationMetadata>[] {
  const groups = groupCommentsByAnchor(path, comments);
  const seen = new Set<string>();
  const annotations: DiffLineAnnotation<ProjectDiffAnnotationMetadata>[] = [];

  for (const [key, group] of groups) {
    seen.add(key);
    const anchor = group[0].anchor;
    if (!anchor) continue;
    annotations.push({
      side: buzzSideToDiffSide(anchor.side),
      lineNumber: anchor.line,
      metadata: {
        anchor,
        comments: group,
        focused: anchorsEqual(anchor, focusedAnchor),
      },
    });
  }

  if (
    focusedAnchor &&
    focusedAnchor.path === path &&
    !seen.has(anchorGroupKey(focusedAnchor))
  ) {
    annotations.push({
      side: buzzSideToDiffSide(focusedAnchor.side),
      lineNumber: focusedAnchor.line,
      metadata: {
        anchor: focusedAnchor,
        comments: [],
        focused: true,
      },
    });
  }

  return annotations.sort(
    (left, right) =>
      left.lineNumber - right.lineNumber ||
      sideOrder(left.side) - sideOrder(right.side),
  );
}

function sideOrder(side: ProjectDiffSide) {
  return side === "deletions" ? 0 : 1;
}

/**
 * Recover the exact Buzz anchor from a Pierre annotation's metadata.
 * Inverse of the mapping performed by {@link buildFileDiffAnnotations}.
 */
export function annotationToBuzzAnchor(
  annotation: DiffLineAnnotation<ProjectDiffAnnotationMetadata>,
): ProjectPullRequestCommentAnchor {
  return annotation.metadata.anchor;
}

/**
 * Map the focused Buzz anchor to the Pierre controlled selected-line range
 * (single line). `null` when there is no focused anchor.
 */
export function focusedAnchorToSelectedRange(
  focusedAnchor: ProjectPullRequestCommentAnchor | null | undefined,
): SelectedLineRange | null {
  if (!focusedAnchor) return null;
  return {
    start: focusedAnchor.line,
    end: focusedAnchor.line,
    side: buzzSideToDiffSide(focusedAnchor.side),
  };
}

/** Structural equality for Buzz comment anchors. */
export function anchorsEqual(
  left: ProjectPullRequestCommentAnchor | null | undefined,
  right: ProjectPullRequestCommentAnchor | null | undefined,
) {
  return Boolean(
    left &&
      right &&
      left.line === right.line &&
      left.path === right.path &&
      left.side === right.side,
  );
}

/**
 * True when a raw patch contains at least one syntactically valid unified
 * hunk header and can be handed to Pierre for rendering. A hunk header must
 * carry numeric old/new ranges (`@@ -<old>[,<count>] +<new>[,<count>] @@`),
 * the same boundary the previous hand-written parser recognized; an optional
 * trailing section/function label after the closing `@@` (e.g. `@@ -12,3
 * +12,4 @@ function renderDiff()`) is allowed. Covers empty/whitespace
 * patches, binary-only diffs ("Binary files differ"), and malformed payloads
 * whose `@@` lines are not real hunk headers — all of which should fall back
 * to the friendly "no textual diff" state.
 */
export function isRenderablePatch(patch: string): boolean {
  return /^@@ -\d+(?:,\d+)? \+\d+(?:,\d+)? @@/m.test(patch);
}

/**
 * Count the rendered body lines of a patch for the truncation banner,
 * matching the previous hand-written renderer's count: every non-metadata
 * line (after stripping `diff --git`, `index`, `---`, `+++` headers) becomes
 * one visible row.
 */
export function patchBodyLineCount(patch: string): number {
  if (!patch.trim()) return 0;
  return patch
    .trimEnd()
    .split("\n")
    .filter(
      (line) =>
        !line.startsWith("diff --git ") &&
        !line.startsWith("index ") &&
        !line.startsWith("--- ") &&
        !line.startsWith("+++ "),
    ).length;
}
