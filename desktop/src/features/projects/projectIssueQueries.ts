import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";
import type { ProjectIssue } from "./projectIssues.mjs";
import { projectIssueEventsToIssues } from "./projectIssues.mjs";
import { fetchIssueAssignmentEvents } from "./projectIssueAssignmentQueries";

type ProjectReference = {
  repoAddress: string;
};

export async function fetchProjectIssues(
  project: ProjectReference,
): Promise<ProjectIssue[]> {
  const issueEventsPromise = relayClient.fetchEvents({
    kinds: [KIND_GIT_ISSUE],
    "#a": [project.repoAddress],
    limit: 200,
  });
  const optionalEventsPromise = Promise.all([
    relayClient.fetchEvents({
      kinds: [
        KIND_GIT_STATUS_OPEN,
        KIND_GIT_STATUS_MERGED,
        KIND_GIT_STATUS_CLOSED,
        KIND_GIT_STATUS_DRAFT,
      ],
      "#a": [project.repoAddress],
      limit: 500,
    }),
    relayClient.fetchEvents({
      kinds: [KIND_TEXT_NOTE],
      "#a": [project.repoAddress],
      limit: 500,
    }),
  ]);
  const issueEvents = await issueEventsPromise;
  const assigneeEventsPromise = fetchIssueAssignmentEvents(
    issueEvents,
    [project.repoAddress],
    relayClient.fetchEvents.bind(relayClient),
  );
  const [[statusEvents, commentEvents], assigneeEvents] = await Promise.all([
    optionalEventsPromise,
    assigneeEventsPromise,
  ]);

  return projectIssueEventsToIssues(
    issueEvents,
    statusEvents,
    commentEvents,
    assigneeEvents,
  );
}
