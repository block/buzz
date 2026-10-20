import { toast } from "sonner";

import { projectCloneErrorPresentation } from "@/features/projects/lib/projectGitError";
import type { ProjectRepoUnavailableReason } from "@/features/projects/lib/projectRepoAvailability";

export function showProjectCloneErrorToast(
  error: unknown,
  cloneUrl?: string | null,
  unavailableReason?: ProjectRepoUnavailableReason,
) {
  const presentation = projectCloneErrorPresentation(
    error,
    cloneUrl,
    unavailableReason,
  );
  toast.error(presentation.title, { description: presentation.description });
}
