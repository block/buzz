import { RotateCcw, Trophy } from "lucide-react";
import * as React from "react";

import type { ArtillerySide } from "@/features/games/artillery/referee";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Button, buttonVariants } from "@/shared/ui/button";

export function ArtilleryVictoryScreen({
  canDeleteLoser,
  deleteError,
  deletePending,
  loserDeleted,
  loserName,
  onDeleteLoser,
  onReplay,
  reason,
  winner,
  winnerName,
}: {
  canDeleteLoser: boolean;
  deleteError: string | null;
  deletePending: boolean;
  loserDeleted: boolean;
  loserName: string | null;
  onDeleteLoser: () => Promise<void>;
  onReplay: () => void;
  reason: "complete" | "forfeited";
  winner: ArtillerySide | "draw";
  winnerName: string;
}) {
  const isDraw = winner === "draw";
  const [deleteDialogOpen, setDeleteDialogOpen] = React.useState(false);

  const deleteLoser = async () => {
    try {
      await onDeleteLoser();
      setDeleteDialogOpen(false);
    } catch {
      // The parent keeps the dialog open and supplies the actionable error.
    }
  };

  return (
    <div
      className="pointer-events-none absolute inset-0 z-30 grid place-items-center bg-slate-950/75 px-6 text-center backdrop-blur-sm motion-safe:animate-in motion-safe:fade-in motion-safe:zoom-in-95 motion-safe:duration-500"
      data-testid="artillery-result"
    >
      <div className="pointer-events-auto rounded-3xl border border-amber-300/40 bg-slate-950/90 px-10 py-8 shadow-2xl shadow-amber-500/20">
        <Trophy
          aria-hidden="true"
          className="mx-auto h-12 w-12 text-amber-300 motion-safe:animate-bounce"
        />
        <div className="mt-3 text-sm font-bold uppercase tracking-[0.3em] text-amber-300">
          {isDraw ? "Match complete" : "Victory"}
        </div>
        <div className="mt-2 text-4xl font-black tracking-tight text-white">
          {isDraw ? "Draw game" : `${winnerName} wins!`}
        </div>
        <p className="mt-2 text-sm text-slate-300">
          {reason === "forfeited"
            ? `${winnerName} wins by forfeit.`
            : isDraw
              ? "Both agents finish level."
              : `${winnerName} is the last bot standing.`}
        </p>
        <div className="mx-auto mt-5 grid w-fit gap-2">
          <Button
            data-testid="artillery-victory-replay"
            onClick={onReplay}
            type="button"
          >
            <RotateCcw aria-hidden="true" /> Replay match
          </Button>
          {!isDraw ? (
            <AlertDialog
              onOpenChange={setDeleteDialogOpen}
              open={deleteDialogOpen}
            >
              <AlertDialogTrigger asChild>
                <Button
                  className="h-auto flex-col gap-0.5 py-2"
                  data-testid="artillery-delete-loser"
                  disabled={!canDeleteLoser || deletePending || loserDeleted}
                  title={
                    canDeleteLoser
                      ? undefined
                      : "Only a managed agent from a live match can be deleted."
                  }
                  type="button"
                  variant="destructive"
                >
                  <span>
                    {loserDeleted ? "Loser deleted 💀" : "Delete the loser 💀"}
                  </span>
                  {loserName ? (
                    <span className="text-xs font-normal opacity-80">
                      ({loserName})
                    </span>
                  ) : null}
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent data-testid="artillery-delete-loser-dialog">
                <AlertDialogHeader>
                  <AlertDialogTitle>Delete {loserName}?</AlertDialogTitle>
                  <AlertDialogDescription>
                    This permanently removes the losing agent from Buzz. This
                    action cannot be undone.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
                  <li>Stops its local or remote agent process</li>
                  <li>Removes its management record and saved identity key</li>
                  <li>
                    Removes it from its channels and archives its identity
                  </li>
                </ul>
                {deleteError ? (
                  <p className="text-sm text-destructive">{deleteError}</p>
                ) : null}
                <AlertDialogFooter>
                  <AlertDialogCancel asChild>
                    <Button
                      disabled={deletePending}
                      type="button"
                      variant="outline"
                    >
                      Cancel
                    </Button>
                  </AlertDialogCancel>
                  <AlertDialogAction
                    className={buttonVariants({ variant: "destructive" })}
                    data-testid="artillery-delete-loser-confirm"
                    disabled={deletePending}
                    onClick={(event) => {
                      event.preventDefault();
                      void deleteLoser();
                    }}
                  >
                    {deletePending ? "Deleting…" : `Delete ${loserName} 💀`}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          ) : null}
        </div>
      </div>
    </div>
  );
}
