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
  visibility: "public" | "private";
  channelId?: string;
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
  if (input.visibility === "private") {
    if (!input.channelId) {
      throw new Error("Select a channel for a private project.");
    }
    tags.push(["buzz-visibility", "private"]);
    tags.push(["buzz-channel", input.channelId]);
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

export type UpdateProjectVisibilityInput = {
  project: Project;
  visibility: "public" | "private";
  channelId?: string;
};

async function updateProjectVisibility(
  input: UpdateProjectVisibilityInput,
): Promise<Project> {
  const identity = await getIdentity();
  if (identity.pubkey.toLowerCase() !== input.project.owner.toLowerCase()) {
    throw new Error("Only the repository owner can change visibility.");
  }
  if (input.visibility === "private" && !input.channelId) {
    throw new Error("Select a channel for a private project.");
  }
  const currentEvents = await relayClient.fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    authors: [input.project.owner],
    "#d": [input.project.dtag],
    limit: 1,
  });
  const current = currentEvents[0];
  if (!current)
    throw new Error("The repository announcement no longer exists.");

  const tags = current.tags.filter(
    (tag) => tag[0] !== "buzz-visibility" && tag[0] !== "buzz-channel",
  );
  if (input.visibility === "private" && input.channelId) {
    tags.push(["buzz-visibility", "private"]);
    tags.push(["buzz-channel", input.channelId]);
  } else if (input.project.channelBindingId) {
    // Public read visibility is independent of the existing push channel.
    // Preserve that binding so changing privacy never changes write policy.
    tags.push(["buzz-channel", input.project.channelBindingId]);
  }
  const event = await signRelayEvent({
    kind: KIND_REPO_ANNOUNCEMENT,
    content: current.content,
    tags,
    createdAt: current.created_at + 1,
  });
  await relayClient.publishEvent(
    event,
    "Timed out updating project visibility.",
    "Failed to update project visibility.",
  );
  return eventToProject(event, getCachedRelayOrigin());
}

export function useUpdateProjectVisibilityMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: updateProjectVisibility,
    onSuccess: (project) => {
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current.map((item) => (item.id === project.id ? project : item)),
      );
      queryClient.setQueryData<Project | null>(
        ["project", project.id],
        project,
      );
    },
  });
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
