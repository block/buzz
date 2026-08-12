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
 * First-run-per-version splash: shows a short changelog grouped by version
 * the first time this install launches a build whose real app version it
 * hasn't recorded as seen yet, then stays dismissed for that version. Fully
 * self-contained — mount once near the top of the authenticated app shell,
 * no props required.
 *
 * Dismissal is explicit-only: the backdrop and Escape key are disabled here
 * (this is meant to actually be read), so "Got it" is the only way out.
 */
export function WhatsNewModal() {
  const { entries, isOpen, onDismiss } = useWhatsNewModal();

  if (entries.length === 0) return null;

  return (
    <Dialog open={isOpen} onOpenChange={() => {}}>
      <DialogContent
        className="sm:max-w-[440px]"
        onEscapeKeyDown={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
        showCloseButton={false}
      >
        <DialogHeader>
          <DialogTitle>What's new</DialogTitle>
          <DialogDescription className="sr-only">
            A short changelog of recent features in this build, grouped by
            version.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          {entries.map((entry) => (
            <div key={entry.version} className="flex flex-col gap-1.5">
              <p className="text-sm font-medium text-muted-foreground">
                0.5.5-{entry.version}
              </p>
              <ul className="list-disc space-y-1 pl-5 text-sm">
                {entry.bullets.map((bullet) => (
                  <li key={bullet}>{bullet}</li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <DialogFooter>
          <Button onClick={onDismiss} variant="default">
            Got it
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
