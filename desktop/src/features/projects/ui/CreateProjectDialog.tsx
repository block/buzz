import type { CreateProjectInput } from "@/features/projects/useCreateProject";
import { Dialog } from "@/shared/ui/dialog";
import { CreateProjectFormContent } from "./CreateProjectFormContent";

type CreateProjectDialogProps = {
  isCreating: boolean;
  onCreate: (input: CreateProjectInput) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
};

/** Modal for creating a project channel. */
export function CreateProjectDialog({
  isCreating,
  onCreate,
  onOpenChange,
  open,
}: CreateProjectDialogProps) {
  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isCreating) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      <CreateProjectFormContent
        active={open}
        isCreating={isCreating}
        onCreate={onCreate}
        onCreated={() => onOpenChange(false)}
      />
    </Dialog>
  );
}
