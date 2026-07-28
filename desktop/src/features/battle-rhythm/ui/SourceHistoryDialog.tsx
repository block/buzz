import * as React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { ImportRevisionInput } from "../data/battleRhythmService";
import { buildRollbackPreview, type RollbackPreview } from "../data/rollback";
import type {
  BattleRhythmRevision,
  BattleRhythmSource,
} from "../domain/contracts";
export function SourceHistoryDialog({
  open,
  onOpenChange,
  ownerPubkey,
  sources,
  revisions,
  onRollback,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  ownerPubkey: string;
  sources: readonly BattleRhythmSource[];
  revisions: readonly BattleRhythmRevision[];
  onRollback: (input: ImportRevisionInput) => Promise<void>;
}) {
  const [preview, setPreview] = React.useState<RollbackPreview>();
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string>();
  async function applyRollback() {
    if (!preview) return;
    setBusy(true);
    setError(undefined);
    try {
      await onRollback(preview.input);
      setPreview(undefined);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Rollback failed.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        onOpenChange(next);
        if (!next) {
          setPreview(undefined);
          setError(undefined);
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Source revisions</DialogTitle>
        </DialogHeader>
        {sources.length ? (
          <div className="grid gap-3">
            {sources.map((source) => {
              const history = revisions
                .filter((revision) => revision.sourceId === source.id)
                .sort(
                  (left, right) =>
                    Date.parse(right.importedAt) - Date.parse(left.importedAt),
                );
              return (
                <section className="rounded border p-3" key={source.id}>
                  <strong className="text-sm">{source.displayName}</strong>
                  <p className="text-xs text-muted-foreground">
                    {source.documentName}
                  </p>
                  <ul className="mt-2 grid gap-2">
                    {history.map((revision) => {
                      const active = revision.id === source.revisionId;
                      return (
                        <li
                          className="flex items-center justify-between gap-3 rounded bg-muted/40 p-2 text-xs"
                          key={revision.id}
                        >
                          <span>
                            Revision {revision.id}
                            <span className="block text-2xs text-muted-foreground">
                              {new Date(revision.importedAt).toLocaleString()} ·{" "}
                              {revision.changes.length} changes
                            </span>
                          </span>
                          {active ? (
                            <span className="text-2xs uppercase text-primary">
                              Active
                            </span>
                          ) : (
                            <button
                              className="rounded border px-2 py-1"
                              onClick={() => {
                                setError(undefined);
                                setPreview(
                                  buildRollbackPreview({
                                    ownerPubkey,
                                    source,
                                    revisions,
                                    targetRevisionId: revision.id,
                                    revisionId: crypto.randomUUID(),
                                    importedAt: new Date().toISOString(),
                                  }),
                                );
                              }}
                              type="button"
                            >
                              Review rollback
                            </button>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                </section>
              );
            })}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            No source revisions have been imported.
          </p>
        )}
        {preview ? (
          <div className="rounded border border-amber-500/50 bg-amber-500/10 p-3 text-sm">
            <strong>Rollback review</strong>
            <p className="mt-1 text-xs">
              Restore revision {preview.targetRevisionId}: {preview.added} added
              · {preview.changed} changed · {preview.removed} removed. This
              publishes a new signed revision and preserves the history above.
            </p>
            <div className="mt-3 flex justify-end gap-2">
              <button
                className="rounded border px-2 py-1"
                onClick={() => setPreview(undefined)}
                type="button"
              >
                Cancel
              </button>
              <button
                className="rounded bg-primary px-2 py-1 text-primary-foreground disabled:opacity-50"
                disabled={busy}
                onClick={applyRollback}
                type="button"
              >
                Apply rollback revision
              </button>
            </div>
          </div>
        ) : null}
        {error ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
