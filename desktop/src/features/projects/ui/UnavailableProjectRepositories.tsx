import { FolderGit2 } from "lucide-react";

import type { Project } from "@/features/projects/hooks";
import { useTranslation } from "@/i18n";

export function UnavailableProjectRepositories({
  project,
}: {
  project: Project;
}) {
  const { t } = useTranslation();
  return (project.unavailableRepositoryAddresses ?? []).map((address) => (
    <div
      className="flex items-center gap-2 rounded-md border border-border/70 px-3 py-2 text-xs text-muted-foreground"
      key={address}
    >
      <FolderGit2 className="h-3.5 w-3.5" />
      <span className="max-w-80 truncate">
        {address.slice(address.indexOf(":", 6) + 1)}
      </span>
      <span>{t("projects.card.unavailable")}</span>
    </div>
  ));
}
