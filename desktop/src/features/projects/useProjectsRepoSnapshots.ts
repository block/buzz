import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  getProjectLocalRepoSnapshot,
  getProjectRepoSnapshot,
} from "@/shared/api/projectGit";
import type { ProjectRepoSnapshot } from "@/shared/api/types";
import type { Project } from "./hooks";

// Remote snapshots are backed by a blobless `git clone` per repository, so the
// overview scan is deliberately throttled and cached for a long time.
const OVERVIEW_SNAPSHOT_CONCURRENCY = 3;

function snapshotHasData(snapshot: ProjectRepoSnapshot | null | undefined) {
  return Boolean(
    snapshot && (snapshot.files.length > 0 || snapshot.latestCommit),
  );
}

/**
 * Local checkouts are instant (no network, no clone) and keep working when
 * the relay's git storage is empty or unreachable, so they are preferred.
 * Only repositories without usable local data fall back to a remote clone.
 */
async function fetchProjectSnapshot(
  project: Project,
  reposDir: string | null | undefined,
): Promise<ProjectRepoSnapshot | null> {
  try {
    const local = await getProjectLocalRepoSnapshot({
      reposDir,
      projectDtag: project.dtag,
      cloneUrl: project.cloneUrls[0] ?? null,
      defaultBranch: project.defaultBranch,
      baseBranch: project.defaultBranch,
    });
    if (snapshotHasData(local?.snapshot)) return local?.snapshot ?? null;
  } catch {
    // Best-effort: fall through to the remote snapshot.
  }

  const cloneUrl = project.cloneUrls[0];
  if (!cloneUrl) return null;
  return getProjectRepoSnapshot({
    cloneUrl,
    defaultBranch: project.defaultBranch,
    baseBranch: project.defaultBranch,
  });
}

async function fetchProjectsRepoSnapshots(
  projects: Project[],
  reposDir: string | null | undefined,
): Promise<Record<string, ProjectRepoSnapshot>> {
  const snapshots: Record<string, ProjectRepoSnapshot> = {};
  const queue = [...projects];

  const workers = Array.from(
    { length: Math.min(OVERVIEW_SNAPSHOT_CONCURRENCY, queue.length) },
    async () => {
      for (;;) {
        const project = queue.shift();
        if (!project) return;
        try {
          const snapshot = await fetchProjectSnapshot(project, reposDir);
          if (snapshot) snapshots[project.id] = snapshot;
        } catch {
          // Best-effort: unreachable or empty repositories are skipped.
        }
      }
    },
  );

  await Promise.all(workers);
  return snapshots;
}

/**
 * Fetches repo snapshots for a set of projects (throttled, failure-tolerant)
 * for workspace-wide aggregates like the overview language breakdown.
 * Prefers local checkouts under `reposDir`; falls back to remote clones.
 * Callers should pre-filter and cap `projects` — up to one git clone per entry.
 */
export function useProjectsRepoSnapshotsQuery(
  projects: Project[],
  reposDir?: string | null,
) {
  const projectIds = React.useMemo(
    () => projects.map((project) => project.id).sort(),
    [projects],
  );

  return useQuery({
    enabled: projects.length > 0,
    queryKey: ["projects", "repo-snapshots", reposDir ?? "default", projectIds],
    queryFn: () => fetchProjectsRepoSnapshots(projects, reposDir),
    staleTime: 15 * 60_000,
    retry: 0,
  });
}
