import { useQuery } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import { KIND_REPO_ANNOUNCEMENT } from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";

export type Project = {
  id: string;
  dtag: string;
  name: string;
  description: string;
  cloneUrls: string[];
  webUrl: string | null;
  owner: string;
  contributors: string[];
  createdAt: number;
};

function getTag(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((t) => t[0] === name)?.[1];
}

function getAllTags(event: RelayEvent, name: string): string[] {
  return event.tags.filter((t) => t[0] === name).map((t) => t[1]);
}

function getCloneUrls(event: RelayEvent): string[] {
  const tag = event.tags.find((t) => t[0] === "clone");
  return tag ? tag.slice(1) : [];
}

function eventToProject(event: RelayEvent): Project {
  const d = getTag(event, "d") ?? event.id;
  const name = getTag(event, "name") || d;
  const description = getTag(event, "description") || event.content || "";
  const cloneUrls = getCloneUrls(event);
  const webUrl = getTag(event, "web") ?? null;
  const contributors = getAllTags(event, "p");

  return {
    id: `${event.pubkey}:${d}`,
    dtag: d,
    name,
    description,
    cloneUrls,
    webUrl,
    owner: event.pubkey,
    contributors,
    createdAt: event.created_at,
  };
}

function dedup(events: RelayEvent[]): RelayEvent[] {
  const best = new Map<string, RelayEvent>();

  for (const e of events) {
    const d = getTag(e, "d") ?? "";
    const key = `${e.pubkey}:${e.kind}:${d}`;
    const prev = best.get(key);

    if (!prev || e.created_at > prev.created_at) {
      best.set(key, e);
    }
  }

  return [...best.values()];
}

async function fetchProjects(): Promise<Project[]> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    limit: 200,
  });

  return dedup(events)
    .map(eventToProject)
    .sort((a, b) => b.createdAt - a.createdAt);
}

async function fetchProject(projectId: string): Promise<Project | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    "#d": [projectId],
    limit: 10,
  });

  const deduped = dedup(events);
  return deduped.length > 0 ? eventToProject(deduped[0]) : null;
}

export const projectsQueryKey = ["projects"] as const;

export function useProjectsQuery() {
  return useQuery({
    queryKey: projectsQueryKey,
    queryFn: fetchProjects,
    staleTime: 60_000,
  });
}

export function useProjectQuery(projectId: string) {
  return useQuery({
    queryKey: ["project", projectId],
    queryFn: () => fetchProject(projectId),
    staleTime: 60_000,
  });
}
