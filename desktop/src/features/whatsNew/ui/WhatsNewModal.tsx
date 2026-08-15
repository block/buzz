import { useWhatsNewModal } from "@/features/whatsNew/useWhatsNewModal";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

/**
 * First-run-per-version splash: what shipped in *this* release, shown once the
 * first time an install launches a version it hasn't recorded as seen.
 *
 * Deliberately scoped to the current version only. It previously rendered
 * every changelog entry ever written, which grew by a section per release and
 * meant a returning user was re-shown things they had already read several
 * times — the reliable way to train people to dismiss the splash unread. The
 * full history lives in Settings → Updates, which is somewhere people can go
 * back to rather than somewhere they have to get past.
 *
 * Dismissal is explicit-only: the backdrop and Escape key are disabled here
 * (this is meant to actually be read), so "Got it" is the only way out.
 */
export function WhatsNewModal() {
  const { entry, isOpen, onDismiss } = useWhatsNewModal();

  if (!entry) return null;

  return (
    <Dialog open={isOpen} onOpenChange={() => {}}>
      <DialogContent
        className="sm:max-w-[440px]"
        onEscapeKeyDown={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
        showCloseButton={false}
      >
        <DialogHeader>
          <DialogTitle>What's new in {entry.version}</DialogTitle>
          <DialogDescription className="sr-only">
            What shipped in this version of Buzz.
          </DialogDescription>
        </DialogHeader>

        <ul className="list-disc space-y-1.5 pl-5 text-sm">
          {entry.bullets.map((bullet) => (
            <li key={bullet}>{bullet}</li>
          ))}
        </ul>

        <p className="text-sm text-muted-foreground">
          Earlier releases are listed under Settings → Updates.
        </p>

        <DialogFooter>
          <Button onClick={onDismiss} variant="default">
            Got it
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
