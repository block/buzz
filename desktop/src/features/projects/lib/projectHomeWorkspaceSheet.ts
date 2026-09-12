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

const WORKSPACE_SHEET_TITLES: Record<ProjectHomeWorkspaceSheetTab, string> = {
  commits: "Commits",
  contributors: "People",
  files: "Files",
  issues: "Tasks",
  prs: "Reviews",
};

export function projectHomeWorkspaceSheetTitle(
  tab: ProjectHomeWorkspaceSheetTab,
): string {
  return WORKSPACE_SHEET_TITLES[tab];
}

/** Repository workspace tab to open when expanding a home-channel sheet. */
export function projectHomeWorkspaceSheetExpandTab(
  tab: ProjectHomeWorkspaceSheetTab,
): ProjectHomeWorkspaceSheetTab {
  return tab;
}
