// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useCreateProjectMutation } from "@/features/projects/useCreateProject";
import { CreateProjectDialog } from "@/features/projects/ui/CreateProjectDialog";

/** Shared project-creation flow for populated and first-run project views. */
export function ProjectCreationDialog({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  useUiLanguage();

  const { goProject } = useAppNavigation();
  const createProjectMutation = useCreateProjectMutation();

  return (
    <CreateProjectDialog
      isCreating={createProjectMutation.isPending}
      onCreate={async (input) => {
        const result = await createProjectMutation.mutateAsync(input);
        if (result.compatibilityWarning) {
          toast.warning(uiT("Created as a standalone project"), {
            description: result.compatibilityWarning,
          });
        } else {
          toast.success(
            uiT('Project "{0}" created.', { p0: result.project.name }),
          );
        }
        await goProject(result.project.id);
      }}
      onOpenChange={onOpenChange}
      open={open}
    />
  );
}
