import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";
import {
  MAX_FEEDBACK_REQUEST_BYTES,
  type ReviewBlock,
} from "../lib/reviewContract";
import type { FeedbackPublishState } from "../lib/reviewOutbox";

export type ReviewCommentPhase =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "discarding" }
  | { kind: "error"; message: string };

type ReviewCommentFormProps = {
  block: ReviewBlock;
  value: string;
  phase: ReviewCommentPhase;
  /** A signed payload exists: the text can no longer change, only be retried. */
  locked: boolean;
  /** The relay already accepted the feedback revision; only the wake remains. */
  feedbackPublished: boolean;
  /** What is known about the relay holding the signed feedback, if signed. */
  publishState: FeedbackPublishState | null;
  /**
   * The relay provably holds nothing for this comment (never sent, or refused
   * and confirmed absent), so it may be discarded. False while ambiguous.
   */
  canDiscard: boolean;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
  onDiscard: () => void;
  /**
   * Why sending a new comment is paused right now (head check pending/failed or
   * a newer revision), or null. Null for a locked draft: its retry is never
   * gated on the head.
   */
  blockedReason: string | null;
};

/**
 * Styling for an action that is unavailable but still focusable. These buttons
 * use `aria-disabled` plus a guarded handler instead of `disabled`: a focused
 * button that becomes `disabled` (for example while a send is in flight) loses
 * keyboard focus to the page.
 */
const UNAVAILABLE_ACTION_CLASS =
  "aria-disabled:pointer-events-none aria-disabled:opacity-50";

/** What a signed comment's Retry will do, by what is known about the relay. */
function lockedHint(
  feedbackPublished: boolean,
  publishState: FeedbackPublishState | null,
): string {
  if (feedbackPublished) {
    return "Your feedback was published. Retry to notify the agent.";
  }
  switch (publishState) {
    case "ambiguous":
      return "Sent, but the relay has not confirmed it. Retry asks the relay and finishes it; it cannot be discarded until the relay confirms it was not stored.";
    case "rejected":
      return "The relay refused this comment. Retry sends the same feedback again, or discard it.";
    case "never_attempted":
      return "This comment is signed but was not sent. Retry sends it, or discard it.";
    default:
      return "This comment is signed. Retry sends the same feedback.";
  }
}

/**
 * The labelled comment form, rendered by trusted chrome outside the artifact
 * document. Escape and Cancel close it (the caller restores focus); the live
 * region announces sending, success, and error states to assistive tech.
 */
export function ReviewCommentForm({
  block,
  value,
  phase,
  locked,
  feedbackPublished,
  publishState,
  canDiscard,
  onChange,
  onSubmit,
  onCancel,
  onDiscard,
  blockedReason,
}: ReviewCommentFormProps) {
  const headingId = React.useId();
  const textareaId = React.useId();
  const hintId = React.useId();
  const errorId = React.useId();
  const blockedId = React.useId();
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);
  const sending = phase.kind === "sending";
  const discarding = phase.kind === "discarding";
  // Sending and discarding both hold the comment: nothing else may start.
  const busy = sending || discarding;
  const empty = value.trim().length === 0;
  const submitUnavailable = busy || empty || Boolean(blockedReason);

  // The one gate every submission path goes through (the Send/Retry button,
  // the form's own submit, and Ctrl/⌘+Enter in the text field), so no path can
  // submit while the button reads unavailable.
  const requestSubmit = () => {
    if (!submitUnavailable) onSubmit();
  };

  // Discard hands the text back for editing and removes its own button when it
  // finishes, so focus moves to the text field first, before that button can
  // unmount under it.
  const handleDiscard = () => {
    if (busy) return;
    onDiscard();
    textareaRef.current?.focus();
  };

  // biome-ignore lint/correctness/useExhaustiveDependencies: focus once per block.
  React.useEffect(() => {
    textareaRef.current?.focus();
  }, [block.id]);

  return (
    <form
      aria-labelledby={headingId}
      className="flex flex-col gap-3 rounded-2xl border border-border/70 bg-muted/30 p-3"
      data-testid="review-comment-form"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onCancel();
        }
      }}
      onSubmit={(event) => {
        event.preventDefault();
        requestSubmit();
      }}
    >
      <div className="min-w-0">
        <h3 className="text-sm font-semibold" id={headingId}>
          Comment on “{block.title}”
        </h3>
        {block.sourceRef ? (
          <p className="truncate font-mono text-xs text-muted-foreground">
            {block.sourceRef}
          </p>
        ) : null}
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs font-medium" htmlFor={textareaId}>
          Your comment
        </label>
        <Textarea
          aria-describedby={
            phase.kind === "error" ? `${hintId} ${errorId}` : hintId
          }
          id={textareaId}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
              event.preventDefault();
              requestSubmit();
            }
          }}
          placeholder="What should change in this block?"
          readOnly={locked || busy}
          ref={textareaRef}
          rows={5}
          value={value}
        />
        <p className="text-xs text-muted-foreground" id={hintId}>
          {locked
            ? lockedHint(feedbackPublished, publishState)
            : `Up to ${MAX_FEEDBACK_REQUEST_BYTES / 1024} KiB. Ctrl or ⌘ + Enter sends.`}
        </p>
      </div>
      {phase.kind === "error" ? (
        <p
          className="text-xs text-destructive"
          data-testid="review-comment-error"
          id={errorId}
        >
          {phase.message}
        </p>
      ) : null}
      {blockedReason ? (
        <p
          className="text-xs text-muted-foreground"
          data-testid="review-comment-blocked"
          id={blockedId}
          role="status"
        >
          {blockedReason}
        </p>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button onClick={onCancel} size="sm" type="button" variant="ghost">
          Cancel
        </Button>
        {locked && canDiscard ? (
          <Button
            aria-disabled={busy || undefined}
            className={UNAVAILABLE_ACTION_CLASS}
            onClick={handleDiscard}
            size="sm"
            type="button"
            variant="outline"
          >
            {discarding ? "Discarding…" : "Discard"}
          </Button>
        ) : null}
        <Button
          aria-describedby={blockedReason ? blockedId : undefined}
          aria-disabled={submitUnavailable || undefined}
          className={UNAVAILABLE_ACTION_CLASS}
          onClick={(event) => {
            // Unavailable but focusable: block the form submit here.
            if (submitUnavailable) event.preventDefault();
          }}
          size="sm"
          type="submit"
        >
          {sending
            ? "Sending…"
            : locked || phase.kind === "error"
              ? "Retry"
              : "Send feedback"}
        </Button>
      </div>
    </form>
  );
}
