import type { RelayEvent } from "@/shared/api/types";
import { KIND_GIT_ISSUE_ASSIGNEE } from "@/shared/constants/kinds";

const MAX_ASSIGNMENT_HEADS_PER_ISSUE = 2;
const RELAY_MAX_PAGE_SIZE = 1_000;
const ISSUE_IDS_PER_QUERY =
  RELAY_MAX_PAGE_SIZE / MAX_ASSIGNMENT_HEADS_PER_ISSUE;

type AssignmentFilter = {
  kinds: number[];
  "#a": string[];
  "#d": string[];
  limit: number;
};

type FetchEvents = (filter: AssignmentFilter) => Promise<RelayEvent[]>;

/**
 * Fetches every authorized NIP-33 assignment head without crossing the
 * relay's 1,000-event page clamp. Each issue has at most two authorized
 * author-scoped heads: one from the issue author and one from the repo owner.
 */
export async function fetchIssueAssignmentEvents(
  issueEvents: RelayEvent[],
  repoAddresses: string[],
  fetchEvents: FetchEvents,
): Promise<RelayEvent[]> {
  if (repoAddresses.length === 0) return [];

  const issueIds = [
    ...new Set(
      issueEvents
        .map((event) => event.id.toLowerCase())
        .filter((id) => /^[a-f0-9]{64}$/.test(id)),
    ),
  ];

  const queries: Array<Promise<RelayEvent[]>> = [];
  for (
    let offset = 0;
    offset < issueIds.length;
    offset += ISSUE_IDS_PER_QUERY
  ) {
    const ids = issueIds.slice(offset, offset + ISSUE_IDS_PER_QUERY);
    queries.push(
      fetchEvents({
        kinds: [KIND_GIT_ISSUE_ASSIGNEE],
        "#a": repoAddresses,
        "#d": ids,
        limit: ids.length * MAX_ASSIGNMENT_HEADS_PER_ISSUE,
      }),
    );
  }

  return (await Promise.all(queries)).flat();
}
