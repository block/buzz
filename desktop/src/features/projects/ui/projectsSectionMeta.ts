import {
  CircleDot,
  FolderGit2,
  Folders,
  GitPullRequest,
  Hash,
  type LucideIcon,
} from "lucide-react";

import type { ProjectsFilter } from "@/features/projects/lib/projectsViewHelpers";
import { i18n } from "@/i18n";

export function projectsSectionTitle(filter: ProjectsFilter) {
  if (filter === "all") return i18n.t("projects.sections.activity");
  if (filter === "prs") return i18n.t("projects.sections.reviews");
  if (filter === "issues") return i18n.t("projects.sections.tasks");
  if (filter === "repositories")
    return i18n.t("projects.sections.repositories");
  if (filter === "channels") return i18n.t("projects.sections.channels");
  return i18n.t("projects.sections.projects");
}

export function projectsSectionIcon(filter: ProjectsFilter): LucideIcon {
  if (filter === "prs") return GitPullRequest;
  if (filter === "issues") return CircleDot;
  if (filter === "repositories") return FolderGit2;
  if (filter === "channels") return Hash;
  return Folders;
}

export function openAppSearch() {
  document
    .querySelector<HTMLButtonElement>('[data-testid="open-search"]')
    ?.click();
}
