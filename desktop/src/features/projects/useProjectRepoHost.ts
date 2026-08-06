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
    canCloneLocally:
      host.kind === "buzz" ||
      (host.kind === "external" && host.host === "github.com"),
    controls: {
      externalUrl: host.kind === "external" ? webUrl : null,
      remoteKind: host.kind === "unresolved" ? undefined : host.kind,
      remoteLabel:
        host.kind === "external"
          ? host.host
          : host.kind === "buzz"
            ? "Buzz"
            : "Remote",
    },
  };
}
