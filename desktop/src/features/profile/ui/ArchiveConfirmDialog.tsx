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
import { useTranslation } from "@/i18n";

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
  const { t } = useTranslation();
  const title = isBot
    ? t("profile.archive-confirm.title-agent")
    : t("profile.archive-confirm.title-identity");
  const subject = isBot
    ? t("profile.archive-confirm.subject-agent")
    : t("profile.archive-confirm.subject-person");

  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent data-testid="archive-confirm-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>
            {t("profile.archive-confirm.hides", { subject })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {/* The list + closing paragraph sit outside AlertDialogDescription on
            purpose — that component renders a <p>, which can't legally contain
            a <ul> or another block <p>. */}
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          <li>{t("profile.archive-confirm.list-search")}</li>
          <li>
            {t("profile.archive-confirm.list-scope-before")}
            <span className="font-medium text-foreground">
              {t("profile.archive-confirm.list-scope-emphasis")}
            </span>
            {t("profile.archive-confirm.list-scope-after")}
          </li>
          <li>{t("profile.archive-confirm.list-restore")}</li>
        </ul>
        {isBot ? (
          <p className="text-sm text-muted-foreground">
            {t("profile.archive-confirm.bot-note")}
          </p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              {t("profile.archive-confirm.cancel")}
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction
            className={buttonVariants({ variant: "secondary" })}
            data-testid="archive-confirm-action"
            disabled={isPending}
            onClick={onConfirm}
          >
            {isPending
              ? t("profile.archive-confirm.archiving")
              : t("profile.archive-confirm.archive")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
