import { useQuery } from "@tanstack/react-query";
import { queryEvents, type NostrEvent } from "@/shared/lib/nostr-client";
import { relayWsUrl } from "@/shared/lib/relay-url";
import { getMockRepo, mockRepos } from "./mock-repos";

export interface Repo {
  id: string;
  name: string;
  description: string;
  cloneUrls: string[];
  webUrl: string | null;
  channelId: string | null;
  owner: string;
  contributors: string[];
  createdAt: number;
}

/** Extract the first value for a given tag name from a Nostr event. */
export function getTag(event: NostrEvent, name: string): string | undefined {
  return event.tags.find((t) => t[0] === name)?.[1];
}

/** Extract all values for a given tag name from a Nostr event. */
function getAllTags(event: NostrEvent, name: string): string[] {
  return event.tags.filter((t) => t[0] === name).map((t) => t[1]);
}

function eventToRepo(event: NostrEvent): Repo {
  const d = getTag(event, "d") ?? event.id;
  const name = getTag(event, "name") || d;
  const description = getTag(event, "description") || event.content || "";
  const cloneUrls = getAllTags(event, "clone");
  const webUrl = getTag(event, "web") ?? null;
  const channelId = getTag(event, "buzz-channel") ?? null;
  const contributors = getAllTags(event, "p");
  const owner = event.pubkey;

  return {
    id: d,
    name,
    description,
    cloneUrls,
    webUrl,
    channelId,
    owner,
    contributors,
    createdAt: event.created_at,
  };
}

/** Deduplicate NIP-33 parameterized replaceable events, keeping the latest per (pubkey, kind, d-tag). */
export function dedup(events: NostrEvent[]): NostrEvent[] {
  const best = new Map<string, NostrEvent>();
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

async function fetchRepos(): Promise<Repo[]> {
  const events = await queryEvents(relayWsUrl(), { kinds: [30617] });
  return dedup(events)
    .map(eventToRepo)
    .sort((a, b) => b.createdAt - a.createdAt);
}

export function useRepos({ enabled = true }: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ["repos"],
    queryFn: fetchRepos,
    enabled,
    staleTime: 60_000,
  });
}

export function useRepo(repoId: string) {
  const mockRepo = getMockRepo(repoId);

  return useQuery({
    queryKey: mockRepo ? ["repos", "mock"] : ["repos"],
    queryFn: mockRepo ? async () => mockRepos : fetchRepos,
    initialData: mockRepo ? mockRepos : undefined,
    staleTime: 60_000,
    select: (repos) => repos.find((r) => r.id === repoId),
  });
}
