import * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { SharedCatchUp } from "@/shared/api/tauriThreadCatchUp";
import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import { Textarea } from "@/shared/ui/textarea";
import {
  CATCH_UP_SECTION_LABELS,
  catchUpCorrectionBytes,
  formatCatchUpTime,
  isCatchUpConflictError,
  MAX_CATCH_UP_CORRECTION_BYTES,
  validateCatchUpCorrection,
} from "./threadCatchUp";
import {
  projectSharedCatchUpPoints,
  type SharedCatchUpPointView,
  sharedCatchUpAttribution,
} from "./threadCatchUpArtifact";
import {
  ThreadCatchUpSources,
  useThreadMessagesById,
} from "./ThreadCatchUpSources";
import type { ThreadCatchUpController } from "./useThreadCatchUp";

type Controller = NonNullable<ThreadCatchUpController>;
type Draft = { text: string; error: string | null; conflict: boolean };
type Label = (pubkey: string) => string;

/**
 * Recaps already shared with this thread. Generated text never changes;
 * each correction is attributed to its signer and kept in history. Drafts
 * live here, keyed by recap and point, so a refresh after a conflict keeps
 * what the person wrote.
 */
export function ThreadCatchUpShared({
  controller,
  currentPubkey,
  profiles,
}: {
  controller: Controller;
  currentPubkey: string;
  profiles?: UserProfileLookup;
}) {
  const { artifactsQuery } = controller;
  const [drafts, setDrafts] = React.useState<ReadonlyMap<string, Draft>>(
    () => new Map(),
  );
  const label = React.useCallback<Label>(
    (pubkey) => resolveUserLabel({ pubkey, currentPubkey, profiles }),
    [currentPubkey, profiles],
  );
  const messagesById = useThreadMessagesById(controller.threadMessages);

  const updateDraft = (key: string, next: Draft | null) =>
    setDrafts((current) => {
      const map = new Map(current);
      if (next) map.set(key, next);
      else map.delete(key);
      return map;
    });

  let content: React.ReactNode;
  if (artifactsQuery.isPending) {
    content = (
      <Skeleton
        aria-label="Loading shared recaps"
        className="h-4 w-2/3"
        role="status"
      />
    );
  } else if (artifactsQuery.isError) {
    content = (
      <Alert variant="destructive">
        <AlertDescription>
          Shared recaps couldn't be loaded. {artifactsQuery.error.message}
        </AlertDescription>
        <Button
          className="mt-2"
          onClick={() => void controller.refreshShared()}
          size="xs"
          type="button"
          variant="outline"
        >
          Try again
        </Button>
      </Alert>
    );
  } else {
    const { recaps, complete, unreadableCount } = artifactsQuery.data;
    content = (
      <>
        {recaps.length === 0 && unreadableCount === 0 && complete ? (
          <p className="text-xs text-muted-foreground">
            No recaps have been shared with this thread.
          </p>
        ) : (
          <ul className="space-y-3">
            {recaps.map((shared) => (
              <SharedRecap
                controller={controller}
                drafts={drafts}
                key={shared.artifactId}
                label={label}
                messagesById={messagesById}
                shared={shared}
                updateDraft={updateDraft}
              />
            ))}
          </ul>
        )}
        {unreadableCount > 0 ? (
          <p
            className="text-2xs text-muted-foreground"
            data-testid="thread-catch-up-unreadable"
          >
            {unreadableCount} shared{" "}
            {unreadableCount === 1 ? "recap" : "recaps"} couldn't be verified
            and {unreadableCount === 1 ? "isn't" : "aren't"} shown.
          </p>
        ) : null}
        {!complete ? (
          <p
            className="text-2xs text-muted-foreground"
            data-testid="thread-catch-up-truncated"
          >
            Showing the most recently changed shared recaps. Older ones aren't
            listed.
          </p>
        ) : null}
      </>
    );
  }

  return (
    <section
      aria-labelledby="thread-catch-up-shared-title"
      className="space-y-2 border-t border-border/40 pt-3"
      data-testid="thread-catch-up-shared"
    >
      <h4
        className="text-xs font-medium text-muted-foreground"
        id="thread-catch-up-shared-title"
      >
        Shared with this thread
      </h4>
      {content}
    </section>
  );
}

function SharedRecap({
  controller,
  drafts,
  label,
  messagesById,
  shared,
  updateDraft,
}: {
  controller: Controller;
  drafts: ReadonlyMap<string, Draft>;
  label: Label;
  messagesById: ReturnType<typeof useThreadMessagesById>;
  shared: SharedCatchUp;
  updateDraft: (key: string, next: Draft | null) => void;
}) {
  const attribution = sharedCatchUpAttribution(shared);
  const views = projectSharedCatchUpPoints(shared);
  return (
    <li
      className="space-y-2 rounded-xl border border-border/50 p-2.5"
      data-artifact-id={shared.artifactId}
      data-testid="thread-catch-up-shared-recap"
    >
      <p
        className="text-2xs text-muted-foreground"
        data-testid="thread-catch-up-attribution"
      >
        {attribution.kind === "sharer"
          ? `Shared by ${label(attribution.pubkey)} · ${formatCatchUpTime(attribution.createdAt)}`
          : "Sharer unknown: earlier history is no longer available."}
        {" · covers "}
        {formatCatchUpTime(shared.recap.since)} –{" "}
        {formatCatchUpTime(shared.recap.until)}
      </p>
      {views.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          No decisions or open questions.
        </p>
      ) : (
        <ul className="space-y-2.5">
          {views.map((view) => {
            const key = `${shared.artifactId}:${view.point.id}`;
            return (
              <SharedPoint
                controller={controller}
                draft={drafts.get(key) ?? null}
                key={key}
                label={label}
                messagesById={messagesById}
                setDraft={(next) => updateDraft(key, next)}
                shared={shared}
                view={view}
              />
            );
          })}
        </ul>
      )}
    </li>
  );
}

function SharedPoint({
  controller,
  draft,
  label,
  messagesById,
  setDraft,
  shared,
  view,
}: {
  controller: Controller;
  draft: Draft | null;
  label: Label;
  messagesById: ReturnType<typeof useThreadMessagesById>;
  setDraft: (next: Draft | null) => void;
  shared: SharedCatchUp;
  view: SharedCatchUpPointView;
}) {
  const [saving, setSaving] = React.useState(false);
  const textareaId = React.useId();
  const latest = view.history[view.history.length - 1];
  const bytes = draft ? catchUpCorrectionBytes(draft.text) : 0;

  const save = async () => {
    if (!draft) return;
    const error = validateCatchUpCorrection(draft.text, view.currentText);
    if (error) {
      setDraft({ ...draft, error, conflict: false });
      return;
    }
    setSaving(true);
    try {
      await controller.correct.mutateAsync({
        artifactId: shared.artifactId,
        prevEventId: shared.eventId,
        pointId: view.point.id,
        text: draft.text,
      });
      setDraft(null);
    } catch (cause) {
      if (isCatchUpConflictError(cause)) {
        // Keep the draft; the reloaded recap supplies the new `prev`.
        setDraft({
          ...draft,
          conflict: true,
          error:
            "Someone changed this recap while you were editing. Your correction wasn't saved; review the latest version below and save again.",
        });
        void controller.refreshShared();
      } else {
        setDraft({
          ...draft,
          conflict: false,
          error:
            `Correction wasn't saved. ${cause instanceof Error ? cause.message : ""}`.trim(),
        });
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <li
      data-point-id={view.point.id}
      data-testid="thread-catch-up-shared-point"
    >
      <p className="text-2xs font-medium text-muted-foreground">
        {CATCH_UP_SECTION_LABELS[view.point.category]}
      </p>
      <p
        className="whitespace-pre-wrap break-words text-sm leading-5"
        data-testid="thread-catch-up-shared-point-text"
      >
        {view.currentText}
      </p>
      {view.corrected && latest.authorPubkey ? (
        <p className="text-2xs text-muted-foreground">
          Corrected by {label(latest.authorPubkey)}
          {latest.createdAt ? ` · ${formatCatchUpTime(latest.createdAt)}` : ""}
        </p>
      ) : null}
      <ThreadCatchUpSources
        bodies={controller.bodies}
        channelId={controller.scope.channelId}
        citedBodies={null}
        messagesById={messagesById}
        rootEventId={controller.scope.rootEventId}
        sourceIds={view.point.sourceIds}
      />
      {view.history.length > 1 ? (
        <details
          className="mt-1 text-2xs"
          data-testid="thread-catch-up-history"
        >
          <summary className="cursor-pointer text-muted-foreground hover:text-foreground">
            History ({view.history.length})
          </summary>
          <ol className="mt-1 space-y-1.5 border-l border-border/60 pl-2.5">
            {view.history.map((entry) => (
              <li key={entry.key}>
                <p className="text-muted-foreground">
                  {entry.kind === "generated"
                    ? "Generated text"
                    : `Correction by ${entry.authorPubkey ? label(entry.authorPubkey) : "unknown"}`}
                  {entry.createdAt
                    ? ` · ${formatCatchUpTime(entry.createdAt)}`
                    : ""}
                </p>
                <p className="whitespace-pre-wrap break-words text-xs">
                  {entry.text}
                </p>
              </li>
            ))}
          </ol>
        </details>
      ) : null}
      {draft ? (
        <form
          className="mt-2 space-y-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <label className="sr-only" htmlFor={textareaId}>
            Corrected text
          </label>
          <Textarea
            aria-describedby={`${textareaId}-count`}
            aria-invalid={draft.error ? true : undefined}
            className="min-h-16 text-sm"
            data-testid="thread-catch-up-correction-input"
            disabled={saving || controller.isArchived}
            id={textareaId}
            onChange={(event) =>
              setDraft({
                text: event.target.value,
                error: null,
                conflict: draft.conflict,
              })
            }
            value={draft.text}
          />
          <p
            className="text-2xs text-muted-foreground"
            id={`${textareaId}-count`}
          >
            {bytes} / {MAX_CATCH_UP_CORRECTION_BYTES} bytes. Plain text; the
            original stays in history.
          </p>
          {draft.error ? (
            <p
              className="text-xs text-destructive"
              data-conflict={draft.conflict ? "true" : undefined}
              data-testid="thread-catch-up-correction-error"
              role="alert"
            >
              {draft.error}
            </p>
          ) : null}
          <div className="flex flex-wrap gap-2">
            <Button
              data-testid="thread-catch-up-correction-save"
              disabled={saving}
              size="xs"
              type="submit"
            >
              {saving ? "Saving…" : "Save correction"}
            </Button>
            <Button
              disabled={saving}
              onClick={() => setDraft(null)}
              size="xs"
              type="button"
              variant="ghost"
            >
              Discard
            </Button>
          </div>
        </form>
      ) : controller.isArchived ? null : (
        <Button
          className="mt-1 h-auto px-0 text-2xs"
          data-testid="thread-catch-up-correct"
          onClick={() =>
            setDraft({ text: view.currentText, error: null, conflict: false })
          }
          type="button"
          variant="link"
        >
          Correct this point
        </Button>
      )}
    </li>
  );
}
