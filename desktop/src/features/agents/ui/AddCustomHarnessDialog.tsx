import { CustomHarnessForm } from "@/features/settings/ui/CustomHarnessForm";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

/**
 * Registers a custom ACP harness from inside an agent dialog, so "New agent"
 * is a complete entry point and not a dead end that sends the user to
 * Settings. Hosts the same `CustomHarnessForm` the harness catalog uses.
 */
export function AddCustomHarnessDialog({
  onOpenChange,
  onSaved,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  /** Called with the id of the harness that was just registered. */
  onSaved: (id: string) => void;
  open: boolean;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="h-[42rem] max-w-2xl"
        contentClassName="flex min-h-0 flex-1 p-0"
        data-testid="add-custom-harness-dialog"
        scrollAreaClassName="flex min-h-0 overflow-hidden px-0"
        title="Add custom harness"
      >
        <CustomHarnessForm
          chromeless
          header={
            <p className="text-sm text-muted-foreground">
              Register any ACP-speaking agent tool as a selectable harness.
            </p>
          }
          onCancel={() => onOpenChange(false)}
          onSaved={(id) => {
            // Dismiss on save as well as cancel — both exits belong to this
            // dialog, so callers only handle the resulting selection.
            onOpenChange(false);
            onSaved(id);
          }}
        />
      </ChooserDialogContent>
    </Dialog>
  );
}
