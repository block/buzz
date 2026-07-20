import type {
  ProjectLocalRepository,
  ProjectLocalRepoSnapshot,
  ProjectRepoCloneResult,
  ProjectRepoDiff,
  ProjectRepoMergeResult,
  ProjectRepoPullResult,
  ProjectRepoPushResult,
  ProjectRepoSnapshot,
  ProjectRepoSyncStatus,
} from "@/shared/api/types";
import { invokeTauri, TauriInvokeError } from "@/shared/api/tauri";

type RawProjectRepoCommit = {
  hash: string;
  short_hash: string;
  author_name: string;
  author_email: string;
  timestamp: number;
  subject: string;
};

function fromRawProjectRepoCommit(commit: RawProjectRepoCommit) {
  return {
    hash: commit.hash,
    shortHash: commit.short_hash,
    authorName: commit.author_name,
    authorEmail: commit.author_email,
    timestamp: commit.timestamp,
    subject: commit.subject,
  };
}

type RawProjectRepoFile = {
  path: string;
  kind: string;
  size: number | null;
  preview_content: string | null;
  last_changed_at: number | null;
  latest_commit: RawProjectRepoCommit | null;
};

type RawProjectRepoContributor = {
  name: string;
  email: string;
  commit_count: number;
  last_commit_at: number;
};

type RawProjectRepoSnapshot = {
  latest_commit: RawProjectRepoCommit | null;
  commits?: RawProjectRepoCommit[];
  files: RawProjectRepoFile[];
  contributors?: RawProjectRepoContributor[];
};

type RawProjectLocalRepoSnapshot = {
  path: string;
  snapshot: RawProjectRepoSnapshot;
};

type RawProjectLocalRepository = {
  name: string;
  path: string;
};

type RawProjectRepoSyncStatus = {
  local_path: string | null;
  local_branch: string | null;
  local_head: string | null;
  local_short_head: string | null;
  remote_branch: string | null;
  remote_head: string | null;
  remote_short_head: string | null;
  merge_base: string | null;
  ahead_count: number;
  behind_count: number;
  has_uncommitted_changes: boolean;
  has_untracked_files: boolean;
  can_push: boolean;
  push_block_reason: string | null;
  can_pull: boolean;
  pull_block_reason: string | null;
};

type RawProjectRepoPushResult = {
  pushed: boolean;
  message: string;
  branch: string;
  commit: string;
  merge_base: string | null;
};

type RawProjectRepoPullResult = {
  pulled: boolean;
  message: string;
};

type RawProjectRepoDiffFile = {
  path: string;
  additions: number;
  deletions: number;
  patch: string;
  truncated: boolean;
};

type RawProjectRepoDiff = {
  files: RawProjectRepoDiffFile[];
  additions: number;
  deletions: number;
};

function fromRawProjectRepoSnapshot(
  snapshot: RawProjectRepoSnapshot,
): ProjectRepoSnapshot {
  return {
    latestCommit: snapshot.latest_commit
      ? fromRawProjectRepoCommit(snapshot.latest_commit)
      : null,
    commits: (snapshot.commits ?? []).map(fromRawProjectRepoCommit),
    files: snapshot.files.map((file) => ({
      path: file.path,
      kind: file.kind,
      size: file.size,
      previewContent: file.preview_content,
      lastChangedAt: file.last_changed_at,
      latestCommit: file.latest_commit
        ? fromRawProjectRepoCommit(file.latest_commit)
        : null,
    })),
    contributors: (snapshot.contributors ?? []).map((contributor) => ({
      name: contributor.name,
      email: contributor.email,
      commitCount: contributor.commit_count,
      lastCommitAt: contributor.last_commit_at,
    })),
  };
}

export type GitIdentity = {
  name: string | null;
  email: string | null;
};

/** The viewer's configured git identity (`git config user.name/user.email`). */
export async function getGitIdentity(): Promise<GitIdentity> {
  return invokeTauri<GitIdentity>("get_git_identity");
}

export async function getProjectRepoSnapshot(input: {
  cloneUrl: string;
  defaultBranch?: string | null;
  baseBranch?: string | null;
  targetRef?: string | null;
  targetCommit?: string | null;
}): Promise<ProjectRepoSnapshot> {
  const snapshot = await invokeTauri<RawProjectRepoSnapshot>(
    "get_project_repo_snapshot",
    {
      cloneUrl: input.cloneUrl,
      defaultBranch: input.defaultBranch ?? null,
      baseBranch: input.baseBranch ?? null,
      targetRef: input.targetRef ?? null,
      targetCommit: input.targetCommit ?? null,
    },
  );
  return fromRawProjectRepoSnapshot(snapshot);
}

export async function getProjectRepoDiff(input: {
  cloneUrl: string;
  defaultBranch?: string | null;
  baseBranch?: string | null;
  targetRef?: string | null;
  targetCommit?: string | null;
}): Promise<ProjectRepoDiff> {
  const diff = await invokeTauri<RawProjectRepoDiff>("get_project_repo_diff", {
    cloneUrl: input.cloneUrl,
    defaultBranch: input.defaultBranch ?? null,
    baseBranch: input.baseBranch ?? null,
    targetRef: input.targetRef ?? null,
    targetCommit: input.targetCommit ?? null,
  });
  return {
    additions: diff.additions,
    deletions: diff.deletions,
    files: diff.files.map((file) => ({
      path: file.path,
      additions: file.additions,
      deletions: file.deletions,
      patch: file.patch,
      truncated: file.truncated,
    })),
  };
}

export async function getProjectLocalRepoDiff(input: {
  reposDir?: string | null;
  projectDtag: string;
  cloneUrl?: string | null;
  defaultBranch?: string | null;
  baseBranch?: string | null;
  baseCommit?: string | null;
  targetCommit?: string | null;
}): Promise<ProjectRepoDiff | null> {
  const diff = await invokeTauri<RawProjectRepoDiff | null>(
    "get_project_local_repo_diff",
    {
      reposDir: input.reposDir ?? null,
      projectDtag: input.projectDtag,
      cloneUrl: input.cloneUrl ?? null,
      defaultBranch: input.defaultBranch ?? null,
      baseBranch: input.baseBranch ?? null,
      baseCommit: input.baseCommit ?? null,
      targetCommit: input.targetCommit ?? null,
    },
  );
  if (!diff) return null;
  return {
    additions: diff.additions,
    deletions: diff.deletions,
    files: diff.files.map((file) => ({
      path: file.path,
      additions: file.additions,
      deletions: file.deletions,
      patch: file.patch,
      truncated: file.truncated,
    })),
  };
}

export async function getProjectLocalRepoSnapshot(input: {
  reposDir?: string | null;
  projectDtag: string;
  cloneUrl?: string | null;
  defaultBranch?: string | null;
  baseBranch?: string | null;
}): Promise<ProjectLocalRepoSnapshot | null> {
  const localSnapshot = await invokeTauri<RawProjectLocalRepoSnapshot | null>(
    "get_project_local_repo_snapshot",
    {
      reposDir: input.reposDir ?? null,
      projectDtag: input.projectDtag,
      cloneUrl: input.cloneUrl ?? null,
      defaultBranch: input.defaultBranch ?? null,
      baseBranch: input.baseBranch ?? null,
    },
  );
  if (!localSnapshot) return null;
  return {
    path: localSnapshot.path,
    snapshot: fromRawProjectRepoSnapshot(localSnapshot.snapshot),
  };
}

export async function listProjectLocalRepositories(input: {
  reposDir?: string | null;
}): Promise<ProjectLocalRepository[]> {
  const repositories = await invokeTauri<RawProjectLocalRepository[]>(
    "list_project_local_repositories",
    {
      reposDir: input.reposDir ?? null,
    },
  );
  return repositories.map((repository) => ({
    name: repository.name,
    path: repository.path,
  }));
}

function fromRawProjectRepoSyncStatus(
  status: RawProjectRepoSyncStatus,
): ProjectRepoSyncStatus {
  return {
    localPath: status.local_path,
    localBranch: status.local_branch,
    localHead: status.local_head,
    localShortHead: status.local_short_head,
    remoteBranch: status.remote_branch,
    remoteHead: status.remote_head,
    remoteShortHead: status.remote_short_head,
    mergeBase: status.merge_base,
    aheadCount: status.ahead_count,
    behindCount: status.behind_count,
    hasUncommittedChanges: status.has_uncommitted_changes,
    hasUntrackedFiles: status.has_untracked_files,
    canPush: status.can_push,
    pushBlockReason: status.push_block_reason,
    canPull: status.can_pull,
    pullBlockReason: status.pull_block_reason,
  };
}

export async function getProjectRepoSyncStatus(input: {
  reposDir?: string | null;
  projectDtag: string;
  cloneUrl: string;
  branchName?: string | null;
  baseBranch?: string | null;
}): Promise<ProjectRepoSyncStatus> {
  const status = await invokeTauri<RawProjectRepoSyncStatus>(
    "get_project_repo_sync_status",
    {
      reposDir: input.reposDir ?? null,
      projectDtag: input.projectDtag,
      cloneUrl: input.cloneUrl,
      branchName: input.branchName ?? null,
      baseBranch: input.baseBranch ?? null,
    },
  );
  return fromRawProjectRepoSyncStatus(status);
}

type RawProjectTerminalResult = {
  path: string;
  cloned: boolean;
};

export async function openProjectTerminal(input: {
  reposDir?: string | null;
  projectDtag: string;
  cloneUrl?: string | null;
  defaultBranch?: string | null;
}): Promise<{ path: string; cloned: boolean }> {
  const result = await invokeTauri<RawProjectTerminalResult>(
    "open_project_terminal",
    {
      reposDir: input.reposDir ?? null,
      projectDtag: input.projectDtag,
      cloneUrl: input.cloneUrl ?? null,
      defaultBranch: input.defaultBranch ?? null,
    },
  );
  return {
    path: result.path,
    cloned: result.cloned,
  };
}

export async function openProjectMergeRecoveryTerminal(input: {
  reposDir?: string | null;
  projectDtag: string;
  targetCloneUrl: string;
  sourceCloneUrl: string;
  targetBranch: string;
  sourceBranch: string;
  expectedCommit: string;
}): Promise<{
  path: string;
  cloned: boolean;
  recoveryRef: string;
  targetRef: string;
}> {
  const result = await invokeTauri<{
    path: string;
    cloned: boolean;
    recoveryRef: string;
    targetRef: string;
  }>("open_project_merge_recovery_terminal", {
    input: {
      ...input,
      reposDir: input.reposDir ?? null,
    },
  });
  return result;
}

export async function pushProjectLocalRepository(input: {
  reposDir?: string | null;
  projectDtag: string;
  cloneUrl: string;
  branchName?: string | null;
  baseBranch?: string | null;
}): Promise<ProjectRepoPushResult> {
  const result = await invokeTauri<RawProjectRepoPushResult>(
    "push_project_local_repository",
    {
      reposDir: input.reposDir ?? null,
      projectDtag: input.projectDtag,
      cloneUrl: input.cloneUrl,
      branchName: input.branchName ?? null,
      baseBranch: input.baseBranch ?? null,
    },
  );
  return {
    pushed: result.pushed,
    message: result.message,
    branch: result.branch,
    commit: result.commit,
    mergeBase: result.merge_base,
  };
}

export async function pullProjectLocalRepository(input: {
  reposDir?: string | null;
  projectDtag: string;
  cloneUrl: string;
  branchName?: string | null;
}): Promise<ProjectRepoPullResult> {
  const result = await invokeTauri<RawProjectRepoPullResult>(
    "pull_project_local_repository",
    {
      reposDir: input.reposDir ?? null,
      projectDtag: input.projectDtag,
      cloneUrl: input.cloneUrl,
      branchName: input.branchName ?? null,
    },
  );
  return {
    pulled: result.pulled,
    message: result.message,
  };
}

export async function cloneProjectRepository(input: {
  reposDir?: string | null;
  projectDtag: string;
  cloneUrl: string;
  defaultBranch?: string | null;
}): Promise<ProjectRepoCloneResult> {
  return invokeTauri<ProjectRepoCloneResult>("clone_project_repository", {
    reposDir: input.reposDir ?? null,
    projectDtag: input.projectDtag,
    cloneUrl: input.cloneUrl,
    defaultBranch: input.defaultBranch ?? null,
  });
}

type RawProjectRepoMergeResult = {
  message: string;
  merge_commit: string;
  status_event: string;
  status_publication_error: string | null;
};

export type ProjectPullRequestMergeRecovery = {
  action: "open_terminal";
  targetBranch: string;
  sourceBranch: string;
};

/** Machine-readable pull-request merge failure returned by the desktop shell. */
export class ProjectPullRequestMergeError extends Error {
  readonly code: string;
  readonly recovery: ProjectPullRequestMergeRecovery | null;

  constructor(
    code: string,
    message: string,
    recovery: ProjectPullRequestMergeRecovery | null,
  ) {
    super(message);
    this.name = "ProjectPullRequestMergeError";
    this.code = code;
    this.recovery = recovery;
  }
}

function mergeErrorPayload(error: unknown): unknown {
  const payload = error instanceof TauriInvokeError ? error.payload : error;
  if (typeof payload !== "string") return payload;
  try {
    return JSON.parse(payload);
  } catch {
    return null;
  }
}

/** Parse a structured native merge error without classifying generic failures. */
export function parseProjectPullRequestMergeError(
  error: unknown,
): ProjectPullRequestMergeError | null {
  const payload = mergeErrorPayload(error);
  if (!payload || typeof payload !== "object") return null;
  const candidate = payload as {
    code?: unknown;
    message?: unknown;
    recovery?: unknown;
  };
  if (
    typeof candidate.code !== "string" ||
    typeof candidate.message !== "string"
  ) {
    return null;
  }
  let recovery: ProjectPullRequestMergeRecovery | null = null;
  if (candidate.recovery !== null && candidate.recovery !== undefined) {
    if (typeof candidate.recovery !== "object") return null;
    const value = candidate.recovery as {
      action?: unknown;
      sourceBranch?: unknown;
      targetBranch?: unknown;
    };
    if (
      value.action !== "open_terminal" ||
      typeof value.targetBranch !== "string" ||
      typeof value.sourceBranch !== "string"
    ) {
      return null;
    }
    recovery = {
      action: value.action,
      sourceBranch: value.sourceBranch,
      targetBranch: value.targetBranch,
    };
  }
  return new ProjectPullRequestMergeError(
    candidate.code,
    candidate.message,
    recovery,
  );
}

export async function mergeProjectPullRequest(input: {
  targetCloneUrl: string;
  sourceCloneUrl: string;
  targetOwner: string;
  repoAddress: string;
  pullRequestId: string;
  pullRequestAuthor: string;
  statusCreatedAt: number;
  targetBranch: string;
  sourceBranch: string;
  expectedCommit: string;
}): Promise<ProjectRepoMergeResult> {
  let result: RawProjectRepoMergeResult;
  try {
    result = await invokeTauri<RawProjectRepoMergeResult>(
      "merge_project_pull_request",
      {
        input,
      },
    );
  } catch (error) {
    throw parseProjectPullRequestMergeError(error) ?? error;
  }
  return {
    message: result.message,
    mergeCommit: result.merge_commit,
    statusEvent: result.status_event,
    statusPublicationError: result.status_publication_error,
  };
}

export async function signProjectPullRequestReviewRequest(input: {
  targetOwner: string;
  repoAddress: string;
  pullRequestId: string;
  reviewers: string[];
  reviewerLabel: string;
}): Promise<void> {
  await invokeTauri<void>("sign_project_pull_request_review_request", {
    input,
  });
}

export async function publishProjectPullRequestMergedStatus(input: {
  targetOwner: string;
  statusEvent: string;
}): Promise<void> {
  await invokeTauri<void>("publish_project_pull_request_merged_status", {
    input,
  });
}
