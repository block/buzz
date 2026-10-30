// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
export const PROJECT_TASK_CATEGORIES = [
  {
    get label() {
      return uiT("Issue");
    },
    value: "issue",
  },
  {
    get label() {
      return uiT("Change request");
    },
    value: "change-request",
  },
  {
    get label() {
      return uiT("Improvement");
    },
    value: "improvement",
  },
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
  return (
    PROJECT_TASK_CATEGORIES.find((option) => option.value === category)
      ?.label ?? uiT("Issue")
  );
}

export function projectTaskUserLabels(labels: string[]) {
  return labels.filter((label) => !isProjectTaskCategory(label));
}
