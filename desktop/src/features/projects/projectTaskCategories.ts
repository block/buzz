import { i18n } from "@/i18n";

export const PROJECT_TASK_CATEGORIES = [
  { value: "issue" },
  { value: "change-request" },
  { value: "improvement" },
] as const;

export type ProjectTaskCategory =
  (typeof PROJECT_TASK_CATEGORIES)[number]["value"];
export type ProjectTaskCategoryFilter = "all" | ProjectTaskCategory;

const PROJECT_TASK_CATEGORY_VALUES = new Set<string>(
  PROJECT_TASK_CATEGORIES.map(({ value }) => value),
);

export function isProjectTaskCategory(
  value: string,
): value is ProjectTaskCategory {
  return PROJECT_TASK_CATEGORY_VALUES.has(value.toLowerCase());
}

export function projectTaskCategoryFromLabels(
  labels: string[],
): ProjectTaskCategory {
  const category = labels
    .map((label) => label.toLowerCase())
    .find(isProjectTaskCategory);
  return category ?? "issue";
}

export function projectTaskCategoryLabel(category: ProjectTaskCategory) {
  if (category === "change-request")
    return i18n.t("projects.task-category.change-request");
  if (category === "improvement")
    return i18n.t("projects.task-category.improvement");
  return i18n.t("projects.task-category.issue");
}

export function projectTaskUserLabels(labels: string[]) {
  return labels.filter((label) => !isProjectTaskCategory(label));
}
