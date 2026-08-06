export type ProjectDetailScreenProps = {
  commitHash?: string;
  projectId: string;
  pullRequestId?: string;
  issueId?: string;
  repoPath?: string;
  repoRef?: string;
  repositoryId?: string;
};

export const PROJECT_DETAIL_PANEL_SEARCH_KEYS = [
  "profile",
  "profileTab",
  "profileView",
] as const;

export const PROJECT_REPOSITORY_SEARCH_KEYS = [
  "repositoryId",
  "repoRef",
  "repoPath",
  "issueId",
  "pullRequestId",
  "commitHash",
] as const;
