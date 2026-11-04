// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import type { Project } from "@/features/projects/hooks";

export function getDiscussionLabel(project: Project) {
  return project.projectChannelId
    ? uiT("Discussion linked")
    : uiT("No discussion");
}
