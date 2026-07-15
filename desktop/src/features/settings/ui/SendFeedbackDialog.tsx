import { Bug, ImageIcon, ThumbsUp, Wrench, X } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useEmojiBurst } from "@/shared/ui/EmojiBurstProvider";
import { Textarea } from "@/shared/ui/textarea";

/** A random heart emoji so repeated bursts vary a little. */
const HEART_BURST_EMOJIS = ["❤️", "🩷", "🧡", "💛", "💚", "💙", "💜", "💖"];

/**
 * Feedback categories. `id` is what we persist in the outbound message; `label`
 * is user-facing. `positive` categories fire the heart-burst emitter on select.
 */
export type FeedbackCategoryId = "bug" | "praise" | "needs-work";

type FeedbackCategory = {
  id: FeedbackCategoryId;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  positive?: boolean;
};

const FEEDBACK_CATEGORIES: readonly FeedbackCategory[] = [
  { id: "bug", label: "Bug", icon: Bug },
  { id: "praise", label: "Praise", icon: ThumbsUp, positive: true },
  { id: "needs-work", label: "Needs work", icon: Wrench },
];

/** Single source of truth for category id → user-facing label. */
export const FEEDBACK_CATEGORY_LABELS: Record<FeedbackCategoryId, string> =
  Object.fromEntries(
    FEEDBACK_CATEGORIES.map((entry) => [entry.id, entry.label]),
  ) as Record<FeedbackCategoryId, string>;

export type SendFeedbackInput = {
  category: FeedbackCategoryId | null;
  includeLogs: boolean;
  message: string;
};

/**
 * "Send feedback" modal.
 *
 * Layout mirrors {@link NewDirectMessageDialog}: a pill row (here, selectable
 * feedback categories in place of profile pills), a generic feedback box with an
 * optional image attachment shown horizontally beside it, and an "Attach
 * diagnostics" checkbox. Selecting a positive category fires the heart-burst
 * emitter.
 *
 * Delivery (upload and private feedback submission) is delegated to `onSubmit`, and
 * image attachment to `onAttachImage`, so this shell stays presentational.
 */
export function SendFeedbackDialog({
  attachedImageUrl,
  isAttaching,
  isPending,
  onAttachImage,
  onOpenChange,
  onRemoveImage,
  onSubmit,
  open,
}: {
  /** Preview URL of the currently-attached image, or null when none. */
  attachedImageUrl: string | null;
  isAttaching: boolean;
  isPending: boolean;
  /** Opens a file picker and uploads; the parent owns the resulting URL. */
  onAttachImage: () => Promise<void>;
  onOpenChange: (open: boolean) => void;
  onRemoveImage: () => void;
  onSubmit: (input: SendFeedbackInput) => Promise<void>;
  open: boolean;
}) {
  const { burstEmoji } = useEmojiBurst();
  const resolvedAttachedImageUrl = attachedImageUrl
    ? rewriteRelayUrl(attachedImageUrl)
    : null;
  const [category, setCategory] = React.useState<FeedbackCategoryId | null>(
    null,
  );
  const [message, setMessage] = React.useState("");
  const [includeLogs, setIncludeLogs] = React.useState(false);
  const [previewOpen, setPreviewOpen] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) {
      setCategory(null);
      setMessage("");
      setIncludeLogs(false);
      setPreviewOpen(false);
      setErrorMessage(null);
    }
  }, [open]);

  function selectCategory(next: FeedbackCategory, event: React.MouseEvent) {
    const alreadySelected = category === next.id;
    setCategory(alreadySelected ? null : next.id);
    if (!alreadySelected && next.positive) {
      const emoji =
        HEART_BURST_EMOJIS[
          Math.floor(Math.random() * HEART_BURST_EMOJIS.length)
        ] ?? "❤️";
      burstEmoji(emoji, event.currentTarget);
    }
  }

  async function attachImage() {
    if (isAttaching) {
      return;
    }
    setErrorMessage(null);
    try {
      await onAttachImage();
    } catch (error) {
      setErrorMessage(
        error instanceof Error ? error.message : "Failed to attach image.",
      );
    }
  }

  async function submitFeedback() {
    if (isPending || isAttaching || message.trim().length === 0) {
      return;
    }
    setErrorMessage(null);
    try {
      await onSubmit({ category, includeLogs, message: message.trim() });
      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error ? error.message : "Failed to send feedback.",
      );
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-xl gap-0 overflow-hidden border-0 px-6 pb-0 pt-6"
        data-testid="send-feedback-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0 pb-5">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>Send feedback</DialogTitle>
            <DialogClose className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring">
              <X className="h-4 w-4" />
              <span className="sr-only">Close</span>
            </DialogClose>
          </div>
          <p
            className="pt-2 text-sm text-muted-foreground"
            data-testid="feedback-privacy-disclosure"
          >
            Feedback is sent privately to this Buzz deployment and is not posted
            to a channel. Attachments are uploaded before you send.
          </p>
        </DialogHeader>

        <form
          className="flex flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            void submitFeedback();
          }}
        >
          {/*
            Category pills — mirror the New DM recipient chips: the same
            rounded-full silhouette with a circular icon slot on the left. When
            a pill is selected, hovering swaps its icon for an X (the same
            avatar→X affordance DM chips use to remove a recipient), signalling
            that clicking deselects it.
          */}
          <div className="flex flex-wrap items-center gap-2 pb-4">
            {FEEDBACK_CATEGORIES.map((entry) => {
              const Icon = entry.icon;
              const selected = category === entry.id;
              return (
                <button
                  aria-label={entry.label}
                  aria-pressed={selected}
                  className={cn(
                    "group/feedback-pill inline-flex items-center gap-2 rounded-full border py-1 pl-1 pr-3 text-xs transition-colors duration-150 ease-out focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
                    selected
                      ? "border-primary/60 bg-primary/10 text-foreground"
                      : "border-border/80 bg-background/80 text-foreground hover:bg-muted/50",
                  )}
                  data-testid={`feedback-category-${entry.id}`}
                  disabled={isPending}
                  key={entry.id}
                  onClick={(event) => selectCategory(entry, event)}
                  type="button"
                >
                  <span className="relative flex h-8 w-8 shrink-0 items-center justify-center">
                    <span
                      className={cn(
                        "flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-150 ease-out",
                        selected
                          ? "bg-primary/20 text-primary group-hover/feedback-pill:opacity-0 group-focus-visible/feedback-pill:opacity-0"
                          : "bg-muted text-muted-foreground",
                      )}
                    >
                      <Icon className="h-4 w-4" />
                    </span>
                    {selected ? (
                      <span className="absolute inset-0 flex items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 transition-opacity duration-150 ease-out group-hover/feedback-pill:opacity-100 group-focus-visible/feedback-pill:opacity-100">
                        <X aria-hidden="true" className="h-4 w-4" />
                      </span>
                    ) : null}
                  </span>
                  <span className="font-medium">{entry.label}</span>
                </button>
              );
            })}
          </div>

          {/* Feedback box + optional image attachment, laid out horizontally. */}
          <div className="flex items-stretch gap-3">
            <Textarea
              className="min-h-32 flex-1 resize-none"
              data-testid="feedback-message"
              disabled={isPending}
              onChange={(event) => {
                setMessage(event.target.value);
                setErrorMessage(null);
              }}
              placeholder="Tell us what went wrong, or share general feedback."
              value={message}
            />

            {resolvedAttachedImageUrl ? (
              <div className="group/attachment relative flex w-32 shrink-0 flex-col overflow-hidden rounded-lg border border-border/70 bg-muted/40">
                <button
                  aria-label="View attached image"
                  className="flex flex-1 flex-col text-left focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                  data-testid="feedback-attachment-thumb"
                  onClick={() => setPreviewOpen(true)}
                  type="button"
                >
                  <img
                    alt="Attached"
                    className="h-20 w-full object-cover"
                    src={resolvedAttachedImageUrl}
                  />
                  <span className="flex items-center gap-1 px-2 py-1.5 text-2xs font-medium text-muted-foreground">
                    <ImageIcon
                      aria-hidden="true"
                      className="h-3 w-3 shrink-0"
                    />
                    <span className="truncate">Attached image</span>
                  </span>
                </button>
                <button
                  aria-label="Remove attachment"
                  className="absolute right-1 top-1 flex h-5 w-5 items-center justify-center rounded-full bg-background/90 text-muted-foreground opacity-0 shadow transition-opacity duration-150 ease-out hover:text-foreground focus-visible:opacity-100 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring group-hover/attachment:opacity-100"
                  data-testid="feedback-attachment-remove"
                  disabled={isPending}
                  onClick={onRemoveImage}
                  type="button"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            ) : (
              <button
                aria-label="Attach image"
                className="flex w-32 shrink-0 flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border/70 bg-muted/20 p-3 text-center text-2xs font-medium text-muted-foreground transition-colors duration-150 ease-out hover:border-muted-foreground/50 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                data-testid="feedback-attach-image"
                disabled={isPending || isAttaching}
                onClick={() => void attachImage()}
                type="button"
              >
                <ImageIcon aria-hidden="true" className="h-5 w-5" />
                {isAttaching ? "Attaching…" : "Attach image"}
              </button>
            )}
          </div>

          {/* Optional environment diagnostics attachment. */}
          <div className="mt-4 space-y-1.5">
            <label
              className="flex w-fit cursor-pointer items-center gap-2 text-sm text-muted-foreground"
              htmlFor="feedback-include-logs"
            >
              <Checkbox
                checked={includeLogs}
                data-testid="feedback-include-logs"
                disabled={isPending}
                id="feedback-include-logs"
                onCheckedChange={(checked) => setIncludeLogs(checked === true)}
              />
              Attach diagnostics
            </label>
            <p className="pl-6 text-xs text-muted-foreground">
              Includes capture time, app version, platform, user agent, and
              language. No application log lines are collected.
            </p>
          </div>

          {errorMessage ? (
            <p
              className="mt-4 text-sm text-destructive"
              data-testid="feedback-error"
            >
              {errorMessage}
            </p>
          ) : null}

          <div className="flex items-center gap-3 py-4">
            <div className="ml-auto flex items-center gap-2">
              <Button
                disabled={isPending}
                onClick={() => onOpenChange(false)}
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                data-testid="feedback-submit"
                disabled={
                  isPending || isAttaching || message.trim().length === 0
                }
                type="submit"
              >
                {isPending ? "Sending…" : "Send feedback"}
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>

      {/* Full-size attachment preview. */}
      {resolvedAttachedImageUrl ? (
        <Dialog onOpenChange={setPreviewOpen} open={previewOpen}>
          <DialogContent
            aria-describedby={undefined}
            className="max-w-4xl border-0 p-2"
            data-testid="feedback-attachment-preview"
          >
            <DialogTitle className="sr-only">Attached image</DialogTitle>
            <img
              alt="Attached"
              className="max-h-[80vh] w-full rounded-lg bg-black/40 object-contain"
              src={resolvedAttachedImageUrl}
            />
          </DialogContent>
        </Dialog>
      ) : null}
    </Dialog>
  );
}
