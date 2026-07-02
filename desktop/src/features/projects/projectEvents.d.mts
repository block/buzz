import type { RelayEvent } from "@/shared/api/types";

export type ProjectEventTag = string[];

export type ProjectFromEvent = {
  id: string;
  dtag: string;
  name: string;
  description: string;
  cloneUrls: string[];
  webUrl: string | null;
  owner: string;
  contributors: string[];
  createdAt: number;
  projectChannelId: string | null;
  status: string;
  defaultBranch: string;
  repoAddress: string;
};

export function isValidRepoId(repoId: string): boolean;
export function normalizeProjectSlug(input: string): string;
export function getTag(event: RelayEvent, name: string): string | undefined;
export function getAllTags(event: RelayEvent, name: string): string[];
export function getCloneUrls(event: RelayEvent): string[];
export function buildProjectCloneUrl(input: {
  relayHttpUrl: string;
  owner: string;
  repoId: string;
}): string;
export function buildRepoAnnouncementTags(input: {
  repoId: string;
  name?: string | null;
  description?: string | null;
  cloneUrls?: string[];
  webUrl?: string | null;
  relays?: string[];
  projectChannelId?: string | null;
  status?: string;
  defaultBranch?: string;
  contributors?: string[];
}): ProjectEventTag[];
export function eventToProject(event: RelayEvent): ProjectFromEvent;
export function dedupProjectEvents(events: RelayEvent[]): RelayEvent[];
export function projectEventsToProjects(
  events: RelayEvent[],
): ProjectFromEvent[];
