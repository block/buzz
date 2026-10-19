import { ArrowUpRight, RefreshCw } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import {
  channelNextUpItems,
  filterChannelNextUpItems,
} from "../lib/channelNextUp";
import type { Project } from "../projectModels";
import { useChannelNextUp } from "../useChannelNextUp";
import { issueStatusClassName } from "./ProjectIssuesPanel";
import { ProjectPanelState } from "./ProjectPanelState";

type PersonFilter = "all" | "mine" | "teammate";
type ChannelNextUpProps = {
  channelId: string;
  projects: Project[];
  pubkey: string;
};
const PERSON_FILTERS = [
  { value: "all", label: "All" },
  { value: "mine", label: "Mine" },
  { value: "teammate", label: "Teammate" },
] as const;

export function ChannelNextUpRefresh({
  channelId,
  projects,
  pubkey,
}: ChannelNextUpProps) {
  const query = useChannelNextUp(channelId, projects, pubkey);
  return (
    <Button
      aria-label="Refresh tasks"
      disabled={query.isFetching}
      onClick={() => void query.refetch()}
      size="icon"
      type="button"
      variant="ghost"
    >
      <RefreshCw
        className={
          query.isFetching ? "h-4 w-4 motion-safe:animate-spin" : "h-4 w-4"
        }
      />
    </Button>
  );
}

/** A channel view of existing tasks, with no independent task state or writes. */
export function ChannelNextUpPanel({
  channelId,
  projects,
  pubkey,
}: ChannelNextUpProps) {
  const { goProject } = useAppNavigation();
  const query = useChannelNextUp(channelId, projects, pubkey);
  const [personFilter, setPersonFilter] = React.useState<PersonFilter>("all");
  const [teammate, setTeammate] = React.useState("");
  const items = React.useMemo(
    () => channelNextUpItems(projects, query.data?.issues ?? [], channelId),
    [channelId, projects, query.data?.issues],
  );
  const statusesComplete = query.data?.statusesComplete ?? false;
  const assignmentsComplete = query.data?.assignmentsComplete ?? false;
  const activeItems = filterChannelNextUpItems(items, null, statusesComplete);
  const people = [
    ...new Set(
      activeItems.flatMap(({ issue }) =>
        issue.assignees.map((key) => key.toLowerCase()),
      ),
    ),
  ].sort();
  const profiles = useUsersBatchQuery(people);
  const effectiveTeammate = people.includes(teammate) ? teammate : "";
  const assignee = assignmentsComplete
    ? personFilter === "mine"
      ? pubkey
      : personFilter === "teammate"
        ? effectiveTeammate || null
        : null
    : null;
  const visibleItems = filterChannelNextUpItems(
    items,
    assignee,
    statusesComplete,
  );
  const choosingTeammate =
    assignmentsComplete && personFilter === "teammate" && !effectiveTeammate;
  const complete = Boolean(
    query.data?.rootsComplete &&
      statusesComplete &&
      assignmentsComplete &&
      !query.error,
  );
  const label = (key: string) =>
    resolveUserLabel({
      pubkey: key,
      currentPubkey: pubkey,
      profiles: profiles.data?.profiles,
    });
  const ownershipLabel = (assignees: string[]) =>
    assignmentsComplete
      ? assignees.length
        ? assignees.map(label).join(", ")
        : "Unassigned"
      : "Assignment unavailable";
  const showRepository =
    new Set(
      projects.flatMap((project) =>
        project.repositories.map((repository) => repository.repoAddress),
      ),
    ).size > 1;

  return (
    <section
      className="space-y-4"
      data-testid="channel-next-up-panel"
      aria-label="Next up tasks"
    >
      <p className="pt-2 text-xs leading-5 text-muted-foreground">
        Active tasks their authors linked to this channel.
      </p>
      <SegmentedControl<PersonFilter>
        className="max-w-full"
        disabled={!assignmentsComplete || query.isPending}
        legend="Filter tasks by person"
        onValueChange={setPersonFilter}
        optionTestIdPrefix="next-up-person"
        options={PERSON_FILTERS}
        testId="next-up-person-filter"
        value={assignmentsComplete ? personFilter : "all"}
      />
      {assignmentsComplete && personFilter === "teammate" ? (
        <label className="block space-y-1.5 text-xs font-medium">
          <span>Teammate</span>
          <select
            className="h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            onChange={(event) => setTeammate(event.target.value)}
            value={effectiveTeammate}
          >
            <option value="">Choose an assignee</option>
            {people.map((key) => (
              <option key={key} value={key}>
                {label(key)}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      {query.isPending ? (
        <BuzzLoadingState label="Loading channel tasks" />
      ) : (
        <>
          {!complete ? (
            <div
              className="space-y-1 rounded-md border border-border p-3 text-xs leading-5"
              role="status"
            >
              <p className="font-medium">
                {query.error
                  ? "Could not refresh channel tasks."
                  : "This task list is incomplete."}
              </p>
              <p className="text-muted-foreground">
                {!query.data
                  ? "Try refreshing to load tasks."
                  : [
                      !query.data.rootsComplete
                        ? "Older tasks may be missing."
                        : "",
                      !statusesComplete
                        ? "Statuses are unavailable; completed tasks may appear."
                        : "",
                      !assignmentsComplete
                        ? "Assignments are unavailable; person filters are paused."
                        : "",
                      query.error ? "Showing the last loaded tasks." : "",
                    ]
                      .filter(Boolean)
                      .join(" ")}
              </p>
            </div>
          ) : null}
          {choosingTeammate ? (
            <ProjectPanelState
              panel={false}
              title="Choose an assignee"
              description="See what a teammate is working on in this channel."
            />
          ) : visibleItems.length === 0 ? (
            <ProjectPanelState
              panel={false}
              title={
                complete
                  ? personFilter === "all"
                    ? "No active tasks in this channel"
                    : "No active tasks for this person"
                  : "No matching tasks loaded"
              }
              description={
                complete
                  ? "You can also browse the full project task list below."
                  : "Refresh to try again, or open the project task list."
              }
            />
          ) : (
            <ul
              className="divide-y divide-border/60"
              aria-label="Channel tasks"
            >
              {visibleItems.map(({ issue, project, repository }) => (
                <li
                  key={`${repository.repoAddress}:${issue.id}`}
                  data-testid="next-up-task"
                  className="py-3 first:pt-0"
                >
                  <button
                    className="group -mx-1 block w-[calc(100%+0.5rem)] space-y-2 rounded-md p-1 text-left hover:bg-muted/30 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                    aria-label={`${issue.title}, ${statusesComplete ? issue.status : "Status unavailable"}, ${assignmentsComplete && issue.assignees.length ? "assigned to " : ""}${ownershipLabel(issue.assignees)}, opens task`}
                    onClick={() =>
                      void goProject(project.id, {
                        issueId: issue.id,
                        repositoryId: repository.id,
                      })
                    }
                    type="button"
                  >
                    <span className="flex items-start justify-between gap-3">
                      <span className="min-w-0 break-words text-sm font-medium leading-5">
                        {issue.title}
                      </span>
                      <ArrowUpRight className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    </span>
                    <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
                      <span
                        className={
                          statusesComplete
                            ? issueStatusClassName(issue.status)
                            : "text-muted-foreground"
                        }
                      >
                        {statusesComplete ? issue.status : "Status unavailable"}
                      </span>
                      <span aria-hidden="true">·</span>
                      <span className="min-w-0 break-words text-muted-foreground">
                        {ownershipLabel(issue.assignees)}
                      </span>
                      {showRepository ? (
                        <>
                          <span aria-hidden="true">·</span>
                          <span className="text-muted-foreground">
                            {repository.name}
                          </span>
                        </>
                      ) : null}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      <div className="space-y-1 border-t border-border/60 pt-3">
        {projects.map((project) => (
          <Button
            className="-mx-2 h-auto w-[calc(100%+1rem)] justify-between gap-2 whitespace-normal px-2 py-2 text-left text-xs"
            key={project.id}
            onClick={() => void goProject(project.id, { tab: "issues" })}
            type="button"
            variant="ghost"
          >
            <span>All tasks in {project.name}</span>
            <ArrowUpRight className="h-3.5 w-3.5 shrink-0" />
          </Button>
        ))}
      </div>
    </section>
  );
}
