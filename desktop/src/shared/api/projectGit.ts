import type { ProjectRepoSnapshot } from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";

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

export async function getProjectRepoSnapshot(input: {
  cloneUrl: string;
  defaultBranch?: string | null;
  baseBranch?: string | null;
}): Promise<ProjectRepoSnapshot> {
  const snapshot = await invokeTauri<RawProjectRepoSnapshot>(
    "get_project_repo_snapshot",
    {
      cloneUrl: input.cloneUrl,
      defaultBranch: input.defaultBranch ?? null,
      baseBranch: input.baseBranch ?? null,
    },
  );
  return fromRawProjectRepoSnapshot(snapshot);
}
