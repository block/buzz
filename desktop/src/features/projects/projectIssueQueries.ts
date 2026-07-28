import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_ISSUE_ASSIGNEE,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";
import type { ProjectIssue } from "./projectIssues.mjs";
import { projectIssueEventsToIssues } from "./projectIssues.mjs";

type ProjectReference = {
  repoAddress: string;
};

export async function fetchProjectIssues(
  project: ProjectReference,
): Promise<ProjectIssue[]> {
  const [issueEvents, statusEvents, commentEvents, assigneeEvents] =
    await Promise.all([
      relayClient.fetchEvents({
        kinds: [KIND_GIT_ISSUE],
        "#a": [project.repoAddress],
        limit: 200,
      }),
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
      relayClient.fetchEvents({
        kinds: [KIND_GIT_ISSUE_ASSIGNEE],
        "#a": [project.repoAddress],
        // kind:32001 replaces by issue ID, so this returns at most one state
        // per issue and remains above the 200-root issue query bound.
        limit: 500,
      }),
    ]);

  return projectIssueEventsToIssues(
    issueEvents,
    statusEvents,
    commentEvents,
    assigneeEvents,
  );
}
