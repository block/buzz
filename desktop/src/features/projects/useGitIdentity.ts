import { useQuery } from "@tanstack/react-query";

import { getGitIdentity } from "@/shared/api/projectGit";

/**
 * The viewer's configured git identity (`git config user.name/user.email`),
 * used to attribute their own commits to their Buzz profile.
 */
export function useGitIdentityQuery() {
  return useQuery({
    queryKey: ["git-identity"],
    queryFn: getGitIdentity,
    // Git config rarely changes while the app runs.
    staleTime: Number.POSITIVE_INFINITY,
    retry: 1,
  });
}
