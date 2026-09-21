import { ArrowLeft, FolderGit2 } from "lucide-react";

import type { Project } from "@/features/projects/hooks";
import { useTranslation } from "@/i18n";
import { Button } from "@/shared/ui/button";
import { UnavailableProjectRepositories } from "./UnavailableProjectRepositories";

type ProjectDetailUnavailableStateProps =
  | {
      kind: "load-error";
      onBack: () => void;
      onRetry: () => void;
    }
  | {
      kind: "not-found";
      onBack: () => void;
    }
  | {
      kind: "repositories-unavailable";
      project: Project;
    };

export function ProjectDetailUnavailableState(
  props: ProjectDetailUnavailableStateProps,
) {
  const { t } = useTranslation();
  if (props.kind === "load-error") {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-red-400">
          {t("projects.detail-unavailable.load-error")}
        </p>
        <div className="flex items-center gap-2">
          <Button onClick={props.onRetry} size="sm" variant="outline">
            {t("projects.shared.retry")}
          </Button>
          <Button onClick={props.onBack} size="sm" variant="ghost">
            <ArrowLeft className="mr-1.5 h-4 w-4" />
            {t("projects.detail-unavailable.back-to-projects")}
          </Button>
        </div>
      </div>
    );
  }

  if (props.kind === "not-found") {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-muted-foreground">
          {t("projects.detail-unavailable.not-found")}
        </p>
        <Button onClick={props.onBack} size="sm" variant="outline">
          <ArrowLeft className="mr-1.5 h-4 w-4" />
          {t("projects.detail-unavailable.back-to-projects")}
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
      <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
      <p className="text-sm font-medium text-foreground">
        {props.project.name}
      </p>
      <p className="text-sm text-muted-foreground">
        {t("projects.detail-unavailable.no-repositories")}
      </p>
      <UnavailableProjectRepositories project={props.project} />
    </div>
  );
}
