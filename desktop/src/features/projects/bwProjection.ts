import type { Project } from "./hooks";
import { fetchProjectsWorkItems } from "./projectWorkItems";
import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { eventToProjectIssue, type ProjectIssue } from "./projectIssues.mjs";

export type BwNotice = {
  event_id: string;
  outcome: string;
  stage: string;
  code: string;
};
export type BwSnapshot = {
  repo: string;
  activation: { policy: string; genesis: string } | null;
  projection: {
    issues: Record<string, string>;
    issue_fields: Record<
      string,
      {
        title: string;
        description: string;
        acceptance_criteria: string[];
        non_goals: string[];
        type: string;
        platform: string | null;
        priority: string;
      }
    >;
    conflicts: string[];
    children: Record<string, string[]>;
    relations: unknown[];
    artifact_verdicts: Record<string, unknown>;
  };
  records: Record<string, RelayEvent>;
  decisions: Record<string, { outcome: string; stage: string; code: string }>;
  notices: Record<string, BwNotice[]>;
};
export type BwIssueView = {
  state: string | null;
  enrolled: boolean;
  nextActor: string;
  notices: BwNotice[];
  snapshot: BwSnapshot;
};
const labels: Record<string, ProjectIssue["status"]> = {
  triage: "Triage",
  backlog: "Backlog",
  ready: "Backlog",
  "in-development": "In Development",
  implemented: "Implemented",
  resolved: "Done",
  rework: "Backlog",
  closed: "Closed",
};

/** Presentation only: Core owns enrollment, state, conflicts and historical acceptance. */
export function mergeBwIssues(
  legacy: ProjectIssue[],
  snapshot: BwSnapshot,
): ProjectIssue[] {
  const byId = new Map(legacy.map((issue) => [issue.id, issue]));
  const ids = new Set([
    ...Object.keys(snapshot.projection.issues),
    ...Object.keys(snapshot.notices),
  ]);
  for (const id of ids) {
    const root = snapshot.records[id];
    if (root?.kind !== 1621) continue;
    const issue = byId.get(id) ?? eventToProjectIssue(root);
    const state = snapshot.projection.issues[id] ?? null;
    const fields = snapshot.projection.issue_fields[id];
    const notices = snapshot.notices[id] ?? [];
    byId.set(id, {
      ...issue,
      title: fields?.title ?? issue.title,
      content: fields?.description ?? issue.content,
      status: state ? (labels[state] ?? "Triage") : "Triage",
      workflowStatus: null,
      currentReview: null,
      assignees: [],
      assigneeOperationHeads: {},
      bw: {
        state,
        enrolled: state !== null,
        notices,
        snapshot,
        nextActor: notices.length
          ? "Waiting for verified history or evidence"
          : state === "resolved" || state === "closed"
            ? "No action required"
            : state === "in-development" || state === "ready"
              ? "Selected writer"
              : state === "implemented"
                ? "Release coordinator"
                : "Triage operator",
      },
    });
  }
  return [...byId.values()].sort(
    (a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id),
  );
}

export function fetchBwSnapshot(repo: string): Promise<BwSnapshot> {
  return invokeTauri("get_project_bw", { repo });
}

/** Use the same Core view for cross-project lists and repository details. */
export async function fetchBwWorkItems(projects: Project[]) {
  const result = await fetchProjectsWorkItems(projects);
  const contexts = new Map(
    projects.flatMap((project) =>
      project.repositories.map(
        (repository) =>
          [repository.repoAddress, { project, repository }] as const,
      ),
    ),
  );
  const groups = await Promise.all(
    [...contexts.entries()].map(async ([repo, context]) => {
      const snapshot = await fetchBwSnapshot(repo);
      const legacy = result.issues.items
        .filter((row) => row.repository.repoAddress === repo)
        .map((row) => row.issue);
      return mergeBwIssues(legacy, snapshot).map((issue) => ({
        ...context,
        issue,
      }));
    }),
  );
  return { ...result, issues: { ...result.issues, items: groups.flat() } };
}
