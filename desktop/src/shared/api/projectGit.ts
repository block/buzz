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

type RawProjectRepoFile = {
  path: string;
  kind: string;
  size: number | null;
  preview_content: string | null;
};

type RawProjectRepoSnapshot = {
  latest_commit: RawProjectRepoCommit | null;
  files: RawProjectRepoFile[];
};

function fromRawProjectRepoSnapshot(
  snapshot: RawProjectRepoSnapshot,
): ProjectRepoSnapshot {
  return {
    latestCommit: snapshot.latest_commit
      ? {
          hash: snapshot.latest_commit.hash,
          shortHash: snapshot.latest_commit.short_hash,
          authorName: snapshot.latest_commit.author_name,
          authorEmail: snapshot.latest_commit.author_email,
          timestamp: snapshot.latest_commit.timestamp,
          subject: snapshot.latest_commit.subject,
        }
      : null,
    files: snapshot.files.map((file) => ({
      path: file.path,
      kind: file.kind,
      size: file.size,
      previewContent: file.preview_content,
    })),
  };
}

export async function getProjectRepoSnapshot(input: {
  cloneUrl: string;
  defaultBranch?: string | null;
}): Promise<ProjectRepoSnapshot> {
  const snapshot = await invokeTauri<RawProjectRepoSnapshot>(
    "get_project_repo_snapshot",
    {
      cloneUrl: input.cloneUrl,
      defaultBranch: input.defaultBranch ?? null,
    },
  );
  return fromRawProjectRepoSnapshot(snapshot);
}
