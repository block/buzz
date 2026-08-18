import type { Project } from "@/features/projects/hooks";

export function getDiscussionLabel(project: Project) {
  return project.projectChannelId ? "Discussion linked" : "No discussion";
}
