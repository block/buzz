// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button, buttonVariants } from "@/shared/ui/button";

// Archive is relay-scoped + reversible (NIP-IA), so this gates with a calm,
// reassuring confirmation rather than a destructive warning. The confirm action
// renders `secondary` to match the trigger and the non-alarming tone — we pass
// the secondary classes straight to `AlertDialogAction` (whose base style is the
// default/primary variant) so tailwind-merge overrides the primary background;
// `asChild` + a nested Button would concatenate both variants and leave the
// primary fill winning on source order.
export function ArchiveConfirmDialog({
  open,
  onOpenChange,
  onConfirm,
  isBot,
  isPending,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
  isBot: boolean;
  isPending: boolean;
}) {
  const title = isBot
    ? uiT("Archive this agent?")
    : uiT("Archive this identity?");
  const subject = isBot ? "this agent" : "this person";

  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent data-testid="archive-confirm-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>
            {uiT("Archiving hides {0} from the space.", { p0: subject })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {/* The list + closing paragraph sit outside AlertDialogDescription on
            purpose — that component renders a <p>, which can't legally contain
            a <ul> or another block <p>. */}
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          <li>
            {uiT(
              "They won't appear in search, autocomplete, or when adding members",
            )}
          </li>
          <li>
            {uiT("This only affects")}{" "}
            <span className="font-medium text-foreground">
              {uiT("this space")}
            </span>{" "}
            {uiT(" — not their account anywhere else")}
          </li>
          <li>{uiT("You can unarchive them at any time to restore them")}</li>
        </ul>
        {isBot ? (
          <p className="text-sm text-muted-foreground">
            {uiT(
              "You can also delete this agent from the profile settings menu if you want to remove the agent instead of hiding it.",
            )}
          </p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              {uiT("Cancel")}
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction
            className={buttonVariants({ variant: "secondary" })}
            data-testid="archive-confirm-action"
            disabled={isPending}
            onClick={onConfirm}
          >
            {isPending ? uiT("Archiving…") : uiT("Archive")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
