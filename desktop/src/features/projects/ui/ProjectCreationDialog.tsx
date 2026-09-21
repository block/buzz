import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useCreateProjectMutation } from "@/features/projects/useCreateProject";
import { CreateProjectDialog } from "@/features/projects/ui/CreateProjectDialog";
import { useTranslation } from "@/i18n";

/** Shared project-creation flow for populated and first-run project views. */
export function ProjectCreationDialog({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const { t } = useTranslation();
  const { goProject } = useAppNavigation();
  const createProjectMutation = useCreateProjectMutation();

  return (
    <CreateProjectDialog
      isCreating={createProjectMutation.isPending}
      onCreate={async (input) => {
        const result = await createProjectMutation.mutateAsync(input);
        if (result.compatibilityWarning) {
          toast.warning(t("sidebar.projects.created-standalone"), {
            description: result.compatibilityWarning,
          });
        } else {
          toast.success(
            t("sidebar.projects.created", { name: result.project.name }),
          );
        }
        await goProject(result.project.id);
      }}
      onOpenChange={onOpenChange}
      open={open}
    />
  );
}
