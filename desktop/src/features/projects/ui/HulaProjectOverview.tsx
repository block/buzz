import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import type { Project } from "@/features/projects/hooks";
import { projectsQueryKey } from "@/features/projects/hooks";
import {
  branchFromGitStatus,
  hulaCheckoutRows,
  parseWorktreePorcelain,
  type WorktreeRecord,
} from "@/features/projects/lib/hulaCheckout";
import { openClawWorkspaceClient } from "@/features/projects/lib/openClawWorkspaceClient";
import { syncMissingHulaRepositories } from "@/features/projects/syncHulaRepositories";
import { Button } from "@/shared/ui/button";
import { isWebBuild } from "@/features/bridge/ui/DesktopData";
import { DesktopRepoCard, isDesktopRepoError, useDesktopRepoDirect } from "./DesktopRepoCard";
import { OverviewRailSection } from "./ProjectOverviewPanel";
import { ProjectRichContent } from "./ProjectRichContent";

/**
 * OpenClaw checkout for one Hula project.
 * Choosing a row selects that repository. It does not check out a branch.
 */
export function HulaProjectOverview({
  children,
  onSelectRepository,
  project,
}: {
  children: React.ReactNode;
  onSelectRepository: (repositoryId: string) => void;
  project: Project;
}) {
  const queryClient = useQueryClient();
  const hulaPath = project.hulaPath ?? "";
  const readmeQuery = useQuery({
    enabled: hulaPath.length > 0,
    queryKey: ["hula-readme", project.id, hulaPath],
    queryFn: () => openClawWorkspaceClient.readFile(`${hulaPath}/README.md`),
  });
  const checkoutQuery = useQuery({
    enabled: hulaPath.length > 0,
    queryKey: [
      "hula-checkout",
      project.id,
      project.repositories
        .map((repository) => repository.hulaPath ?? "")
        .join("|"),
    ],
    queryFn: () => loadCheckoutRows(project),
  });
  const syncKey = `${project.id}:${project.repositoryAddresses.length}`;
  const syncQuery = useQuery({
    enabled: hulaPath.length > 0,
    queryKey: ["hula-sync", syncKey],
    queryFn: () => syncMissingHulaRepositories(project),
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const added = syncQuery.data ?? 0;

  React.useEffect(() => {
    if (added > 0) {
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
    }
  }, [added, queryClient]);

  const { live } = useDesktopRepoDirect();
  // Web: the checkout is read from your Mac over Direct. Without it, a calm card instead of "unknown" rows or red errors.
  const needsDesktop =
    isWebBuild() &&
    (!live || isDesktopRepoError(syncQuery.error) || isDesktopRepoError(readmeQuery.error));
  const rows = needsDesktop
    ? { repos: [], worktrees: [] }
    : (checkoutQuery.data ?? { repos: [], worktrees: [] });
  const readme = readmeQuery.data;

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-6 px-4 py-4">
      <OverviewRailSection title="Repositories">
        {needsDesktop ? (
          <DesktopRepoCard compact error={syncQuery.error ?? readmeQuery.error} what="these repos" />
        ) : rows.repos.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {checkoutQuery.isLoading
              ? "Reading the OpenClaw checkout…"
              : "No checkout is recorded yet."}
          </p>
        ) : (
          <ul className="space-y-1">
            {rows.repos.map((row) => (
              <li key={row.repositoryId}>
                <button
                  className="w-full rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted/40"
                  data-testid="hula-repo-row"
                  onClick={() => onSelectRepository(row.repositoryId)}
                  type="button"
                >
                  {row.name} · {row.branch}
                </button>
              </li>
            ))}
          </ul>
        )}
      </OverviewRailSection>
      {rows.worktrees.length > 0 ? (
        <OverviewRailSection title="Worktrees">
          <ul className="space-y-1">
            {rows.worktrees.map((row) => (
              <li key={`${row.repositoryId}:${row.path}`}>
                <button
                  className="w-full rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted/40"
                  data-testid="hula-worktree-row"
                  onClick={() => onSelectRepository(row.repositoryId)}
                  type="button"
                >
                  {row.path} · {row.repoName} · {row.branch}
                </button>
              </li>
            ))}
          </ul>
        </OverviewRailSection>
      ) : null}
      {syncQuery.isError && !needsDesktop && !isDesktopRepoError(syncQuery.error) ? (
        <div className="space-y-2">
          <p className="text-sm text-destructive">
            {syncQuery.error instanceof Error
              ? syncQuery.error.message
              : "Could not refresh repositories."}
          </p>
          <Button
            data-testid="hula-sync-retry"
            onClick={() => {
              void syncQuery.refetch();
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            Retry
          </Button>
        </div>
      ) : null}
      {readmeQuery.isSuccess && typeof readme === "string" ? (
        <ProjectRichContent content={readme} />
      ) : (
        children
      )}
    </div>
  );
}

async function loadCheckoutRows(project: Project) {
  const repos = [];
  for (const repository of project.repositories) {
    if (!repository.hulaPath) continue;
    repos.push({
      repositoryId: repository.id,
      name: repository.name,
      hulaPath: repository.hulaPath,
      worktrees: await worktreesFor(repository.hulaPath),
    });
  }
  return hulaCheckoutRows(repos);
}

async function worktreesFor(hulaPath: string): Promise<WorktreeRecord[]> {
  try {
    const listed = await openClawWorkspaceClient.exec(
      ["git", "worktree", "list", "--porcelain"],
      hulaPath,
    );
    const records = parseWorktreePorcelain(listed.stdout);
    if (records.length > 0) return records;
    const status = await openClawWorkspaceClient.exec(
      ["git", "status", "-sb"],
      hulaPath,
    );
    const branch = branchFromGitStatus(status.stdout);
    return [
      {
        path: hulaPath,
        head: null,
        branch,
        detached: branch === null,
        prunable: false,
      },
    ];
  } catch {
    return [];
  }
}
