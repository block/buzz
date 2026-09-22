import { i18n } from "@/i18n";

export const PROJECT_HOME_WORKSPACE_SHEET_TABS = [
  "issues",
  "prs",
  "commits",
  "files",
  "contributors",
] as const;

export type ProjectHomeWorkspaceSheetTab =
  (typeof PROJECT_HOME_WORKSPACE_SHEET_TABS)[number];

export function isProjectHomeWorkspaceSheetTab(
  value: string | undefined,
): value is ProjectHomeWorkspaceSheetTab {
  return (
    value != null &&
    (PROJECT_HOME_WORKSPACE_SHEET_TABS as readonly string[]).includes(value)
  );
}

export function projectHomeWorkspaceSheetTitle(
  tab: ProjectHomeWorkspaceSheetTab,
): string {
  if (tab === "commits") return i18n.t("projects.sections.commits");
  if (tab === "contributors") return i18n.t("projects.home.people");
  if (tab === "files") return i18n.t("projects.sections.files");
  if (tab === "issues") return i18n.t("projects.sections.tasks");
  return i18n.t("projects.sections.reviews");
}

/** Repository workspace tab to open when expanding a home-channel sheet. */
export function projectHomeWorkspaceSheetExpandTab(
  tab: ProjectHomeWorkspaceSheetTab,
): ProjectHomeWorkspaceSheetTab {
  return tab;
}
