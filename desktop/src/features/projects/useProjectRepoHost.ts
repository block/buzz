import type { Repository } from "@/features/projects/hooks";
import {
  type ProjectRepoHost,
  projectRepoHostForRepository,
} from "@/features/projects/lib/projectRepoHost";
import { isSafeUrl } from "@/shared/lib/url";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";

export function useProjectRepoHost(
  repository: Repository | null | undefined,
): ProjectRepoHost {
  return projectRepoHostForRepository(repository, useRelayOrigin());
}

export function useProjectRepoPresentation(
  repository: Repository | null | undefined,
) {
  const host = useProjectRepoHost(repository);
  const webUrl =
    repository?.webUrl && isSafeUrl(repository.webUrl)
      ? repository.webUrl
      : null;

  return {
    host,
    webUrl,
    // The backend enforces the exact trusted-origin allowlist before cloning.
    // Keep the affordance available for configured GitLab/self-hosted remotes
    // instead of hiding every external host except github.com.
    canCloneLocally: host.kind !== "unresolved",
    controls: {
      externalUrl: host.kind === "external" ? webUrl : null,
      remoteKind: host.kind === "unresolved" ? undefined : host.kind,
      remoteLabel: host.kind === "external" ? host.host : "Remote",
    },
  };
}
