import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  eventToProject,
  fetchProjects,
  type Project,
  projectsQueryKey,
} from "@/features/projects/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { KIND_REPO_ANNOUNCEMENT } from "@/shared/constants/kinds";

export type CreateProjectInput = {
  name: string;
  description?: string;
  cloneUrl?: string;
  webUrl?: string;
};

function projectDtagFromName(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** Publishes a NIP-34 repo announcement so the project appears on the relay. */
async function createProject(input: CreateProjectInput): Promise<Project> {
  const name = input.name.trim();
  if (!name) {
    throw new Error("Project name is required.");
  }
  const dtag = projectDtagFromName(name);
  if (!dtag) {
    throw new Error("Project name must include letters or numbers.");
  }

  const identity = await getIdentity();
  const existing = await fetchProjects();
  const ownerPubkey = identity.pubkey.toLowerCase();
  if (
    existing.some(
      (project) =>
        project.owner.toLowerCase() === ownerPubkey && project.dtag === dtag,
    )
  ) {
    throw new Error(`You already have a project named "${dtag}".`);
  }

  const description = input.description?.trim() ?? "";
  const tags: string[][] = [
    ["d", dtag],
    ["name", name],
  ];
  if (description) {
    tags.push(["description", description]);
  }
  const cloneUrl = input.cloneUrl?.trim();
  if (cloneUrl) {
    tags.push(["clone", cloneUrl]);
  }
  const webUrl = input.webUrl?.trim();
  if (webUrl) {
    tags.push(["web", webUrl]);
  }

  const event = await signRelayEvent({
    kind: KIND_REPO_ANNOUNCEMENT,
    content: description,
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out creating project.",
    "Failed to create project.",
  );

  return eventToProject(event, getCachedRelayOrigin());
}

/** Mutation that creates a project and inserts it into the projects cache. */
export function useCreateProjectMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: createProject,
    onSuccess: (project) => {
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) => [
        project,
        ...current,
      ]);
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
    },
  });
}
