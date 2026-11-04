// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import type { Repository } from "@/features/projects/hooks";
import { projectCloneErrorPresentation } from "@/features/projects/lib/projectGitError";
import { openProjectTerminal } from "@/shared/api/projectGit";

export function projectTerminalLabel(hasLocalCheckout: boolean) {
  return hasLocalCheckout
    ? uiT("Open in Terminal")
    : uiT("Clone & open in Terminal");
}

/**
 * Opens the OS terminal at a project's local checkout, cloning first when
 * only a remote exists. Handles the clone progress/success/error toasts and
 * refreshes project queries after a clone so local-checkout state updates.
 */
export function useOpenProjectTerminal(reposDir?: string | null) {
  const queryClient = useQueryClient();

  return React.useCallback(
    async (
      project: Repository,
      options: { branch?: string | null; hasLocalCheckout: boolean },
    ) => {
      const toastId = options.hasLocalCheckout
        ? undefined
        : toast.loading(uiT("Cloning {0}…", { p0: project.name }));
      try {
        const result = await openProjectTerminal({
          reposDir,
          projectDtag: project.dtag,
          cloneUrl: project.cloneUrls[0] ?? null,
          defaultBranch: options.branch ?? project.defaultBranch ?? null,
        });
        if (result.cloned) {
          toast.success(uiT("Cloned to {0}", { p0: result.path }), {
            id: toastId,
          });
          void queryClient.invalidateQueries({
            queryKey: ["project", project.id],
          });
          void queryClient.invalidateQueries({ queryKey: ["projects"] });
        } else if (toastId !== undefined) {
          toast.dismiss(toastId);
        }
      } catch (error) {
        const presentation = options.hasLocalCheckout
          ? {
              get title() {
                return uiT("Couldn’t open terminal");
              },
              get description() {
                return uiT(
                  "Buzz could not open this checkout in your configured terminal.",
                );
              },
            }
          : projectCloneErrorPresentation(error, project.cloneUrls[0]);
        toast.error(presentation.title, {
          description: presentation.description,
          id: toastId,
        });
      }
    },
    [queryClient, reposDir],
  );
}
