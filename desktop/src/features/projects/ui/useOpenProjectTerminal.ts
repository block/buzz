import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import type { Repository } from "@/features/projects/hooks";
import { projectCloneErrorPresentation } from "@/features/projects/lib/projectGitError";
import { i18n } from "@/i18n";
import { openProjectTerminal } from "@/shared/api/projectGit";

export function projectTerminalLabel(hasLocalCheckout: boolean) {
  return hasLocalCheckout
    ? i18n.t("projects.terminal.open-in-terminal")
    : i18n.t("projects.terminal.clone-and-open");
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
        : toast.loading(
            i18n.t("projects.terminal.cloning", { name: project.name }),
          );
      try {
        const result = await openProjectTerminal({
          reposDir,
          projectDtag: project.dtag,
          cloneUrl: project.cloneUrls[0] ?? null,
          defaultBranch: options.branch ?? project.defaultBranch ?? null,
        });
        if (result.cloned) {
          toast.success(
            i18n.t("projects.terminal.cloned-to", { path: result.path }),
            {
              id: toastId,
            },
          );
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
              title: i18n.t("projects.terminal.open-failed-title"),
              description: i18n.t("projects.terminal.open-failed-description"),
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
