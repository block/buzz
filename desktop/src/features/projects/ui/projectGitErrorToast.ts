import { toast } from "sonner";

import { projectCloneErrorPresentation } from "@/features/projects/lib/projectGitError";

export function showProjectCloneErrorToast(
  error: unknown,
  cloneUrl?: string | null,
) {
  const presentation = projectCloneErrorPresentation(error, cloneUrl);
  toast.error(presentation.title, { description: presentation.description });
}
