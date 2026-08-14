import { PatchDiff, type DiffLineAnnotation } from "@pierre/diffs/react";
import type { AnnotationSide, OnDiffLineEnterLeaveProps } from "@pierre/diffs";
import { MessageSquarePlus } from "lucide-react";
import * as React from "react";
import type { CSSProperties } from "react";

import type {
  ProjectPullRequestComment,
  ProjectPullRequestCommentAnchor,
} from "@/features/projects/projectPullRequests.mjs";
import {
  annotationToBuzzAnchor,
  anchorsEqual,
  buildFileDiffAnnotations,
  buzzSideToDiffSide,
  diffSideToBuzzSide,
  isRenderablePatch,
  patchBodyLineCount,
  type ProjectDiffAnnotationMetadata,
} from "@/features/projects/lib/projectDiffAnnotations";
import {
  createFocusOneShotState,
  createHoverAnchorCache,
  focusAnnotationRow,
  focusedAnchorKey,
  includeActiveDiffAnchor,
  markFocusSucceeded,
  nextFocusAttempt,
  recordHoveredLine,
  resolveGutterAnchor,
  selectedRangeForFile,
} from "@/features/projects/lib/projectDiffFocus";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ProjectRepoDiffFile } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { resolveShikiThemeName } from "@/shared/theme/theme-loader";
import { ProjectPullRequestInlineCommentThread } from "./ProjectPullRequestInlineComments";

/**
 * Inline-comment controls contract shared with the Projects changed-files
 * panel. Mirrors the panel's `InlineCommentControls` shape so the adapter can
 * reuse the existing thread/form and add-comment actions without reaching
 * into panel internals.
 */
export type ProjectDiffInlineComments = {
  activeAnchor: ProjectPullRequestCommentAnchor | null;
  canRequestChanges: boolean;
  comments: ProjectPullRequestComment[];
  isSending: boolean;
  onCancel: () => void;
  onStart: (anchor: ProjectPullRequestCommentAnchor) => void;
  onSubmit: (
    anchor: ProjectPullRequestCommentAnchor,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    decision?: "request-changes",
  ) => Promise<unknown>;
  profiles?: UserProfileLookup;
};

const MONO_FONT_STACK =
  'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace';

/**
 * Buzz-owned adapter around Pierre's {@link PatchDiff} for one changed file.
 *
 * Accepts the existing raw per-file patch and the surrounding application
 * state (focused anchor, inline-comment controls) and renders the diff
 * through `@pierre/diffs` with Buzz's presentation choices:
 *
 * - unified layout, classic indicators, line numbers, scroll overflow
 * - metadata hunk separators, no library file header (Buzz renders its own)
 * - no unchanged-line expansion, explicit main-thread rendering
 * - resolved Buzz Shiki theme (Buzz aliases map to github-light/github-dark)
 * - existing inline threads/forms via `renderAnnotation`
 * - keyboard-accessible add-comment action via the gutter utility
 * - focused-line scroll/focus through the public `onPostRender` lifecycle
 *   hook, resolved by {@link focusAnnotationRow}
 *
 * Empty/malformed patches keep the friendly fallback and the truncation
 * banner is preserved above the rendered body.
 */
export function ProjectDiffBody({
  file,
  focusedAnchor,
  inlineComments,
  className,
}: {
  file: ProjectRepoDiffFile;
  focusedAnchor?: ProjectPullRequestCommentAnchor | null;
  inlineComments?: ProjectDiffInlineComments;
  className?: string;
}) {
  const { themeName, isDark } = useTheme();
  const resolvedTheme = resolveShikiThemeName(themeName);

  const lineAnnotations = React.useMemo(
    () =>
      includeActiveDiffAnchor(
        buildFileDiffAnnotations(
          file.path,
          inlineComments?.comments ?? [],
          focusedAnchor,
        ),
        file.path,
        inlineComments?.activeAnchor,
      ),
    [
      file.path,
      focusedAnchor,
      inlineComments?.comments,
      inlineComments?.activeAnchor,
    ],
  );

  const selectedLines = React.useMemo(
    () => selectedRangeForFile(file.path, focusedAnchor),
    [file.path, focusedAnchor],
  );

  // Only scroll/focus once per focused anchor change, not on every post
  // render pass (the hook fires on mount and every update). The key includes
  // the path so switching files at the same side/line is a new focus target.
  // One-shot focused-line lifecycle: remember a key only after a successful
  // focus, clear it when the file-scoped key goes null, and never skip a
  // genuinely new key. A failed lookup stays retryable on a later public
  // `onPostRender` update.
  const focusOneShotRef = React.useRef(createFocusOneShotState());
  const handlePostRender = React.useCallback(
    (node: HTMLElement) => {
      const key = focusedAnchorKey(file.path, focusedAnchor);
      const { state, attempt } = nextFocusAttempt(focusOneShotRef.current, key);
      // `attempt` is only true when the key is non-null, which implies the
      // focused anchor exists and matches this file — the extra guards only
      // satisfy type narrowing.
      if (!attempt || !focusedAnchor || !key) {
        focusOneShotRef.current = state;
        return;
      }
      const succeeded = focusAnnotationRow(
        node,
        buzzSideToDiffSide(focusedAnchor.side),
        focusedAnchor.line,
      );
      focusOneShotRef.current = succeeded ? markFocusSucceeded(key) : state;
    },
    [file.path, focusedAnchor],
  );

  const renderAnnotation = React.useCallback(
    (annotation: DiffLineAnnotation<ProjectDiffAnnotationMetadata>) => {
      if (!inlineComments) return null;
      const anchor = annotationToBuzzAnchor(annotation);
      const isActive = anchorsEqual(inlineComments.activeAnchor, anchor);
      return (
        <ProjectPullRequestInlineCommentThread
          activeAnchor={isActive ? anchor : null}
          canRequestChanges={inlineComments.canRequestChanges}
          comments={annotation.metadata.comments}
          isSending={inlineComments.isSending}
          onCancel={inlineComments.onCancel}
          onSubmit={(content, mentionPubkeys, mediaTags, decision) =>
            inlineComments.onSubmit(
              anchor,
              content,
              mentionPubkeys,
              mediaTags,
              decision,
            )
          }
          profiles={inlineComments.profiles}
        />
      );
    },
    [inlineComments],
  );

  const gutterButtonRef = React.useRef<HTMLButtonElement | null>(null);

  // Last exact { lineNumber, side } received from Pierre's public
  // `onLineEnter`, scoped to this file. The gutter action resolves the live
  // `getHoveredLine()` first and falls back to this cache only when the live
  // getter is absent (e.g. the pointer transitions from the code row into the
  // floating utility, or keyboard activation).
  const hoverAnchorCacheRef = React.useRef(createHoverAnchorCache());

  // The gutter utility is a single floating button the library moves onto the
  // hovered/selected line. Keep its accessible name truthful about the exact
  // line it will open, updated through the public interaction lifecycle.
  const updateGutterLabel = React.useCallback(
    (line: number | null, side: AnnotationSide | null) => {
      const button = gutterButtonRef.current;
      if (!button) return;
      if (line == null || side == null) {
        button.setAttribute("aria-label", `Add line comment on ${file.path}`);
        button.setAttribute("title", "Comment on this line");
        return;
      }
      const buzzSide = diffSideToBuzzSide(side);
      button.setAttribute(
        "aria-label",
        `Comment on ${file.path} ${buzzSide} line ${line}`,
      );
      button.setAttribute(
        "title",
        `Comment on ${file.path} ${buzzSide} line ${line}`,
      );
    },
    [file.path],
  );

  // A file switch must never let a stale hovered line survive: reset the
  // cache and restore the generic accessible label. `updateGutterLabel` is
  // memoized on `file.path`, so depending on it expresses the file-switch
  // lifecycle exactly once per rendered file.
  React.useEffect(() => {
    hoverAnchorCacheRef.current = createHoverAnchorCache();
    updateGutterLabel(null, null);
  }, [updateGutterLabel]);

  const renderGutterUtility = React.useCallback(
    (
      getHoveredLine: () =>
        | { lineNumber: number; side: "deletions" | "additions" }
        | undefined,
    ) => {
      if (!inlineComments) return null;
      return (
        <button
          aria-label={`Add line comment on ${file.path}`}
          className="relative z-[4] pointer-events-auto flex h-5 w-5 items-center justify-center rounded text-muted-foreground hover:bg-primary hover:text-primary-foreground focus-visible:opacity-100 focus-visible:outline-hidden"
          data-testid="project-diff-add-comment"
          onClick={() => {
            const anchor = resolveGutterAnchor(
              hoverAnchorCacheRef.current,
              file.path,
              getHoveredLine(),
            );
            if (!anchor) return;
            inlineComments.onStart({
              line: anchor.lineNumber,
              path: file.path,
              side: diffSideToBuzzSide(anchor.side),
            });
          }}
          onFocus={() => {
            // Keyboard path: live getter first, then the cached public
            // onLineEnter anchor; keep the accessible label exact.
            const anchor = resolveGutterAnchor(
              hoverAnchorCacheRef.current,
              file.path,
              getHoveredLine(),
            );
            if (anchor) {
              updateGutterLabel(anchor.lineNumber, anchor.side);
            }
          }}
          ref={gutterButtonRef}
          title="Comment on this line"
          type="button"
        >
          <MessageSquarePlus className="h-3.5 w-3.5" />
        </button>
      );
    },
    [file.path, inlineComments, updateGutterLabel],
  );

  if (!isRenderablePatch(file.patch)) {
    return (
      <div className="bg-muted/20 px-4 py-4 text-sm text-muted-foreground">
        No textual diff is available for this file.
      </div>
    );
  }

  return (
    <div
      className={cn("bg-background/70", className)}
      data-testid="project-diff-body"
    >
      {file.truncated ? (
        <div className="border-border/40 border-b bg-amber-500/10 px-4 py-2 text-amber-600 dark:text-amber-400">
          Large diff truncated — showing the first{" "}
          {patchBodyLineCount(file.patch)} lines. Use a local checkout to review
          the full change.
        </div>
      ) : null}
      <PatchDiff
        disableWorkerPool
        lineAnnotations={lineAnnotations}
        options={{
          diffStyle: "unified",
          diffIndicators: "classic",
          disableFileHeader: true,
          disableLineNumbers: false,
          enableGutterUtility: inlineComments != null,
          expandUnchanged: false,
          hunkSeparators: "metadata",
          onLineEnter: (props: OnDiffLineEnterLeaveProps) => {
            // Cache the last exact public onLineEnter anchor for this file.
            hoverAnchorCacheRef.current = recordHoveredLine(
              file.path,
              props.lineNumber,
              props.annotationSide,
            );
            updateGutterLabel(props.lineNumber, props.annotationSide);
          },
          onPostRender: (node) => handlePostRender(node),
          overflow: "scroll",
          theme: {
            light: resolvedTheme,
            dark: resolvedTheme,
          },
          themeType: isDark ? "dark" : "light",
        }}
        patch={file.patch}
        renderAnnotation={renderAnnotation}
        renderGutterUtility={renderGutterUtility}
        selectedLines={selectedLines}
        style={
          {
            "--diffs-font-size": "0.75rem",
            "--diffs-line-height": "1.25rem",
            "--diffs-font-family": MONO_FONT_STACK,
          } as CSSProperties
        }
      />
    </div>
  );
}
