import type { RelayEvent } from "@/shared/api/types";
import { collectWithConcurrency } from "@/shared/api/concurrency";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_ISSUE_ASSIGNEE,
} from "@/shared/constants/kinds";
import { verifyEvent } from "nostr-tools/pure";

const RELAY_MAX_PAGE_SIZE = 1_000;
const MAX_FILTERS_PER_REQ = 10;
const ASSIGNMENT_REQ_CONCURRENCY = 4;
const REPO_ADDRESS_PATTERN = /^30617:([a-f0-9]{64}):([A-Za-z0-9._-]{1,64})$/;

type AssignmentFilter = {
  kinds: number[];
  authors: string[];
  "#a": string[];
  "#d": string[];
  limit: number;
};

type FetchEvents = (
  filters: AssignmentFilter | AssignmentFilter[],
) => Promise<RelayEvent[]>;

type WriterGroup = {
  author: string;
  repoAddress: string;
  issueIds: Set<string>;
};

function validRepoOwner(repoAddress: string): string | null {
  const match = REPO_ADDRESS_PATTERN.exec(repoAddress);
  const repoId = match?.[2] ?? "";
  return match && !repoId.startsWith(".") && !repoId.includes("..")
    ? match[1]
    : null;
}

function addWriterGroup(
  groups: Map<string, WriterGroup>,
  author: string,
  repoAddress: string,
  issueId: string,
) {
  const key = `${repoAddress}\u0000${author}`;
  const group = groups.get(key) ?? {
    author,
    repoAddress,
    issueIds: new Set<string>(),
  };
  group.issueIds.add(issueId);
  groups.set(key, group);
}

/**
 * Fetches every authorized NIP-33 assignment head without allowing unrelated
 * author heads to consume the relay limit. One NIP-01 REQ carries exact
 * author/issue filters for each verified root's author and repository owner.
 * Requests respect the relay's ten-filter NIP-11 boundary and use bounded
 * concurrency when many distinct issue authors require separate filters.
 */
export async function fetchIssueAssignmentEvents(
  issueEvents: RelayEvent[],
  repoAddresses: string[],
  fetchEvents: FetchEvents,
): Promise<RelayEvent[]> {
  if (repoAddresses.length === 0) return [];

  const allowedRepos = new Set(repoAddresses);
  const groups = new Map<string, WriterGroup>();
  for (const issue of issueEvents) {
    if (
      typeof issue?.id !== "string" ||
      typeof issue.pubkey !== "string" ||
      !Array.isArray(issue.tags)
    ) {
      continue;
    }
    const issueId = issue.id.toLowerCase();
    const author = issue.pubkey.toLowerCase();
    const repoTags = issue.tags.filter(
      (tag) => Array.isArray(tag) && tag[0] === "a",
    );
    const repoAddress = repoTags.length === 1 ? repoTags[0][1] : undefined;
    const repoOwner = repoAddress ? validRepoOwner(repoAddress) : null;
    if (
      issue.kind !== KIND_GIT_ISSUE ||
      !/^[a-f0-9]{64}$/.test(issueId) ||
      !/^[a-f0-9]{64}$/.test(author) ||
      !repoAddress ||
      !allowedRepos.has(repoAddress) ||
      !repoOwner
    ) {
      continue;
    }
    try {
      if (!verifyEvent(issue)) continue;
    } catch {
      continue;
    }
    addWriterGroup(groups, author, repoAddress, issueId);
    addWriterGroup(groups, repoOwner, repoAddress, issueId);
  }

  const filters: AssignmentFilter[] = [];
  for (const { author, repoAddress, issueIds } of groups.values()) {
    const ids = [...issueIds];
    for (let offset = 0; offset < ids.length; offset += RELAY_MAX_PAGE_SIZE) {
      const chunk = ids.slice(offset, offset + RELAY_MAX_PAGE_SIZE);
      filters.push({
        kinds: [KIND_GIT_ISSUE_ASSIGNEE],
        authors: [author],
        "#a": [repoAddress],
        "#d": chunk,
        limit: chunk.length,
      });
    }
  }

  if (filters.length === 0) return [];

  const requestBatches: AssignmentFilter[][] = [];
  for (let offset = 0; offset < filters.length; offset += MAX_FILTERS_PER_REQ) {
    requestBatches.push(filters.slice(offset, offset + MAX_FILTERS_PER_REQ));
  }
  const events = await collectWithConcurrency(
    requestBatches,
    ASSIGNMENT_REQ_CONCURRENCY,
    fetchEvents,
  );
  return events.flat();
}
