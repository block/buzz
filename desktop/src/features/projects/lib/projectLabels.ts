import type { Project } from "@/features/projects/hooks";
import { i18n } from "@/i18n";

export function getDiscussionLabel(project: Project) {
  return project.projectChannelId
    ? i18n.t("projects.labels.discussion-linked")
    : i18n.t("projects.labels.discussion-none");
}
