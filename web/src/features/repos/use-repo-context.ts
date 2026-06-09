/**
 * Shared resolver for the `(owner, repoName, defaultRef)` triple every
 * repo-scoped page needs. Combines the NIP-34 announcement (`useRepo`) with
 * the refs query (`useRepoRefs`) so callers don't duplicate that wiring.
 *
 * `defaultRef` falls back to `"main"` until refs load, matching the
 * pre-existing behaviour in `RepoDetailPage`.
 */
import { useRepo } from "./use-repos";
import { useRepoRefs } from "./use-repo-refs";

export interface RepoContext {
  owner: string;
  repoName: string;
  defaultRef: string;
  isLoading: boolean;
  error: Error | null;
}

export function useRepoContext(repoId: string): RepoContext {
  const {
    data: repo,
    isLoading: repoLoading,
    error: repoError,
  } = useRepo(repoId);
  const { data: refs, isLoading: refsLoading } = useRepoRefs(repoId);

  return {
    owner: repo?.owner ?? "",
    repoName: repo?.id ?? "",
    defaultRef: refs?.head?.ref ?? "main",
    isLoading: repoLoading || refsLoading,
    error: (repoError as Error | null) ?? null,
  };
}
