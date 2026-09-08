import * as React from "react";

import { homeRepositoriesToBind } from "@/features/projects/lib/projectCollection";
import type { Project } from "@/features/projects/projectModels";
import { useAttachProjectRepositoryMutation } from "@/features/projects/useAttachProjectRepository";
import { relayClient } from "@/shared/api/relayClient";
import { KIND_PROJECT_ANNOUNCEMENT } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Allows automatic healing only from relay-validated owner project data. */
export function canHealProjectHomeRepositories({
  identityPubkey,
  project,
  projectDataIsAuthoritative,
}: {
  identityPubkey?: string;
  project: Pick<Project, "owner" | "repositories">;
  projectDataIsAuthoritative: boolean;
}): boolean {
  return Boolean(
    projectDataIsAuthoritative &&
      identityPubkey &&
      normalizePubkey(identityPubkey) === normalizePubkey(project.owner) &&
      project.repositories.length > 0,
  );
}

/**
 * When the signed-in user owns this project, bind absorbed home-channel
 * repositories onto the `kind:30621` so Overview, other clients, and NIP-MP
 * agree. Agents can announce a repo with `--channel` but cannot edit the
 * owner's project event.
 */
export function useHealProjectHomeRepositories(
  project: Project,
  projectDataIsAuthoritative: boolean,
  identityPubkey?: string,
) {
  const attachMutation = useAttachProjectRepositoryMutation();
  const attemptedRef = React.useRef(new Set<string>());
  const mutateAsync = attachMutation.mutateAsync;

  React.useEffect(() => {
    if (
      !canHealProjectHomeRepositories({
        identityPubkey,
        project,
        projectDataIsAuthoritative,
      })
    ) {
      return;
    }

    let cancelled = false;
    void (async () => {
      const liveHeads = await relayClient.fetchEvents({
        authors: [project.owner],
        kinds: [KIND_PROJECT_ANNOUNCEMENT],
        limit: 1,
        "#d": [project.dtag],
      });
      if (cancelled) return;
      const liveHead = liveHeads[0];
      if (!liveHead) return;
      const signedAddresses = liveHead.tags
        .filter((tag) => tag[0] === "a" && tag[1])
        .map((tag) => tag[1] as string);
      const pending = homeRepositoriesToBind(project, signedAddresses);
      for (const repository of pending) {
        const key = `${project.projectAddress}:${repository.repoAddress}`;
        if (attemptedRef.current.has(key)) continue;
        attemptedRef.current.add(key);
        try {
          await mutateAsync({ project, repository });
        } catch {
          // Already bound, raced, or not owner-writable this session.
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [identityPubkey, mutateAsync, project, projectDataIsAuthoritative]);
}
