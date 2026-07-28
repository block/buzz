import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { BattleRhythmSource } from "../domain/contracts";
export function SourceHistoryDialog({
  open,
  onOpenChange,
  sources,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sources: readonly BattleRhythmSource[];
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Source revisions</DialogTitle>
        </DialogHeader>
        {sources.length ? (
          <ul className="grid gap-2">
            {sources.map((source) => (
              <li className="rounded border p-3 text-sm" key={source.id}>
                <strong>{source.displayName}</strong>
                <span className="ml-2 text-2xs text-muted-foreground">
                  Revision {source.revisionId}
                </span>
                <p className="text-xs text-muted-foreground">
                  {source.documentName} · imported{" "}
                  {new Date(source.importedAt).toLocaleString()}
                </p>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">
            No source revisions have been imported.
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}
