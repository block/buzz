import {
  Circle,
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleX,
  MessageSquare,
  Tag,
  type LucideIcon,
  User,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import {
  type ProjectIssue,
  type Repository as Project,
  useCreateProjectIssueCommentMutation,
  useProjectIssuesQuery,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { entityDiscussionQuery } from "@/features/projects/lib/discussionChannels";
import { selectionItemFromTask } from "@/features/projects/lib/projectSelection";
import { issueShareLink } from "@/features/projects/lib/projectShareLinks";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import {
  projectTaskCategoryLabel,
  projectTaskUserLabels,
} from "@/features/projects/projectTaskCategories";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { IssueAssigneeFacepile, IssueAssigneesRow } from "./IssueAssigneesRow";
import { DiscussedInChannels } from "./DiscussionChannels";
import { ProjectIssueCommentTimeline } from "./ProjectIssueCommentTimeline";
import { ProjectOriginReference } from "./ProjectOriginReference";
import {
  ProjectDetailMetaList,
  ProjectDetailMetaPills,
  ProjectDetailMetaRow,
} from "./ProjectDetailMeta";
import { ProjectDetailSection } from "./ProjectDetailSection";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";
import { ProjectRichContent } from "./ProjectRichContent";
import { ShareLinkButton } from "./ShareLinkButton";
import { PROJECT_DETAIL_READING_COLUMN_CLASS } from "./projectPanelStyles";
import {
  ProjectStatusProgressIcon,
  type ProjectStatusProgressState,
} from "./ProjectStatusProgressIcon";
import { ProjectWorkItemGroup } from "./ProjectWorkItemGroup";
import { ProjectWorkItemRow } from "./ProjectWorkItemRow";
import { ProjectPanelState } from "./ProjectPanelState";

export function issueStatusClassName(status: ProjectIssue["status"]) {
  if (status === "Triage" || status === "In Progress") return "text-amber-500";
  if (status === "Backlog") return "text-muted-foreground";
  if (status === "In Review") return "text-green-500";
  if (status === "Done") return "text-purple-400";
  if (status === "Closed") return "text-destructive";
  return "text-muted-foreground";
}

function issueStatusVisual(status: ProjectIssue["status"]): {
  className: string;
  icon: LucideIcon;
  progress: ProjectStatusProgressState;
} {
  if (status === "Done") {
    return {
      className: "text-purple-400",
      icon: CircleCheck,
      progress: "completed",
    };
  }
  if (status === "Closed") {
    return {
      className: "text-destructive",
      icon: CircleX,
      progress: "canceled",
    };
  }
  if (status === "Backlog") {
    return {
      className: issueStatusClassName(status),
      icon: Circle,
      progress: "queued",
    };
  }
  if (status === "Triage") {
    return {
      className: issueStatusClassName(status),
      icon: CircleDashed,
      progress: "queued",
    };
  }
  return {
    className: issueStatusClassName(status),
    icon: CircleDot,
    progress: status === "In Review" ? "review" : "started",
  };
}

const ISSUE_STATUS_ORDER: readonly ProjectIssue["status"][] = [
  "In Review",
  "In Progress",
  "Triage",
  "Backlog",
  "Done",
  "Closed",
];

export type ProjectIssuePanelItem = {
  issue: ProjectIssue;
  project: Project;
};

function issueMembers(
  project: Project,
  issue: ProjectIssue,
  profiles?: UserProfileLookup,
): ChannelMember[] {
  return [
    ...new Set([
      project.owner,
      issue.author,
      ...project.contributors,
      ...issue.recipients,
    ]),
  ].map((pubkey) => {
    const profile = profiles?.[normalizePubkey(pubkey)];
    return {
      pubkey,
      role: "member" as const,
      isAgent: profile?.isAgent === true,
      joinedAt: new Date(0).toISOString(),
      displayName:
        profile?.displayName?.trim() || profile?.nip05Handle?.trim() || null,
    };
  });
}

function issueSelectionItem(project: Project, issue: ProjectIssue) {
  return selectionItemFromTask({
    author: issue.author,
    channelId: issue.channelId ?? project.channelId,
    id: issue.id,
    shareLink: issueShareLink(issue),
    title: issue.title,
  });
}

function IssueRow({
  issue,
  onOpen,
  profiles,
  project,
  rangeItems,
}: {
  issue: ProjectIssue;
  onOpen: () => void;
  profiles?: UserProfileLookup;
  project: Project;
  rangeItems: ReturnType<typeof issueSelectionItem>[];
}) {
  const authorProfile = profiles?.[normalizePubkey(issue.author)];
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });
  const status = issueStatusVisual(issue.status);

  return (
    <ProjectWorkItemRow
      eventId={issue.id}
      identifier={`#${issue.id.slice(0, 8)}`}
      identifierTitle="View task"
      onOpen={onOpen}
      selection={{
        item: issueSelectionItem(project, issue),
        rangeItems,
      }}
      statusIcon={
        <ProjectStatusProgressIcon
          aria-label={issue.status}
          className={`h-3.5 w-3.5 shrink-0 ${status.className}`}
          state={status.progress}
        />
      }
      testId="project-issue-row"
      title={issue.title}
      trailing={
        <>
          <span
            className="hidden w-24 shrink-0 truncate text-right text-xs text-muted-foreground/60 md:block"
            data-testid="project-issue-row-category"
          >
            {projectTaskCategoryLabel(issue.category)}
          </span>
          <span className="flex w-20 shrink-0 items-center justify-end gap-1">
            <span
              className="flex h-5 w-5 shrink-0 items-center justify-center"
              data-testid="project-issue-creator"
              title={`Created by ${authorLabel}`}
            >
              <ProfileIdentityButton
                avatarClassName="shrink-0"
                avatarSize="xs"
                avatarUrl={authorProfile?.avatarUrl ?? null}
                isAgent={authorProfile?.isAgent === true}
                label={authorLabel}
                pubkey={issue.author}
                showLabel={false}
              />
            </span>
            <span
              className="flex min-w-5 shrink-0 justify-end text-muted-foreground/45"
              data-testid="project-issue-assignee-cell"
            >
              {issue.assignees.length > 0 ? (
                <IssueAssigneeFacepile
                  assignees={issue.assignees}
                  profiles={profiles}
                />
              ) : (
                <span
                  className="flex h-5 w-5 items-center justify-center rounded-full border border-border/70 bg-muted/35"
                  data-testid="project-issue-assignee-placeholder"
                  title="Unassigned"
                >
                  <User aria-hidden="true" className="h-3 w-3" />
                  <span className="sr-only">Unassigned</span>
                </span>
              )}
            </span>
          </span>
          <span className="flex w-8 shrink-0 justify-end">
            <button
              aria-label={
                issue.comments.length > 0
                  ? `View ${issue.comments.length} comments`
                  : "View comments"
              }
              className={`flex items-center gap-1 rounded-md text-xs hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring ${
                issue.comments.length > 0
                  ? "text-muted-foreground/60"
                  : "text-muted-foreground/45"
              }`}
              data-testid="project-issue-comments"
              onClick={onOpen}
              type="button"
            >
              <MessageSquare className="h-3.5 w-3.5" />
              {issue.comments.length}
            </button>
          </span>
          <span
            className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground/55 sm:block"
            data-testid="project-issue-row-date"
            title={new Date(issue.createdAt * 1_000).toLocaleString()}
          >
            {relativeTime(issue.createdAt)}
          </span>
        </>
      }
    />
  );
}

/** Full issue conversation and comment composer. */
export function ProjectIssueDetail({
  issue,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  const commentMutation = useCreateProjectIssueCommentMutation(project);
  const members = React.useMemo(
    () => issueMembers(project, issue, profiles),
    [issue, profiles, project],
  );
  const handleCommentSubmit = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      try {
        await commentMutation.mutateAsync({
          content,
          issue,
          mediaTags,
          mentionPubkeys,
        });
        toast.success("Comment posted.");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to post comment.",
        );
        throw error;
      }
    },
    [commentMutation, issue],
  );
  const identityQuery = useIdentityQuery();
  const status = issueStatusVisual(issue.status);
  const labels = projectTaskUserLabels(issue.labels);
  const viewerPubkey = identityQuery.data?.pubkey;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isAuthor = viewer === normalizePubkey(issue.author);
  const isOwner = viewer === normalizePubkey(project.owner);
  const isManagedAgentOwner = useIsManagedAgent(project.owner) === true;
  const canAssignOthers =
    Boolean(viewer) && (isAuthor || isOwner || isManagedAgentOwner);

  return (
    <div
      className={PROJECT_DETAIL_READING_COLUMN_CLASS}
      data-project-detail-panel
      data-testid="project-issue-detail"
    >
      <header className="space-y-2 px-6 pb-3 pt-5">
        <h3 className="line-clamp-2 text-lg font-semibold leading-6 text-foreground">
          {issue.title}{" "}
          <span className="font-normal text-muted-foreground">
            #{issue.id.slice(0, 8)}
          </span>
          <ShareLinkButton
            className="ml-1 inline-flex h-7 w-7 align-text-bottom"
            label="Copy task link"
            link={issueShareLink(issue)}
            testId="project-issue-copy-link"
          />
        </h3>
        <p className="flex flex-wrap items-center gap-x-1 gap-y-1 text-xs text-muted-foreground">
          <span>Task created</span>
          <span
            className="shrink-0 whitespace-nowrap"
            title={new Date(issue.createdAt * 1_000).toLocaleString()}
          >
            {relativeTime(issue.createdAt)}
          </span>
          <ProjectOriginReference
            agentName={issue.originAgentName}
            channelId={issue.channelId}
          />
        </p>
      </header>
      <ProjectDetailMetaList>
        <ProjectDetailMetaRow icon={status.icon} label="Status">
          <span className={`font-medium ${status.className}`}>
            {issue.status}
          </span>
        </ProjectDetailMetaRow>
        <ProjectDetailMetaRow icon={CircleDot} label="Category">
          {projectTaskCategoryLabel(issue.category)}
        </ProjectDetailMetaRow>
        {issue.assignees.length > 0 || viewer ? (
          <ProjectDetailMetaRow icon={User} label="Assignees">
            <IssueAssigneesRow
              canAssignOthers={canAssignOthers}
              issue={issue}
              profiles={profiles}
              project={project}
              signAsManagedOwner={isManagedAgentOwner && !isOwner}
              viewerPubkey={viewer}
            />
          </ProjectDetailMetaRow>
        ) : null}
        {labels.length > 0 ? (
          <ProjectDetailMetaRow icon={Tag} label="Labels">
            <ProjectDetailMetaPills labels={labels} />
          </ProjectDetailMetaRow>
        ) : null}
      </ProjectDetailMetaList>
      {issue.content ? (
        <ProjectDetailSection defaultOpen title="Description">
          <ProjectRichContent content={issue.content} tags={issue.tags} />
        </ProjectDetailSection>
      ) : null}
      <ProjectDetailSection defaultOpen title="Activity">
        <div className="space-y-3">
          <DiscussedInChannels
            entityLabel="this task"
            originChannelId={issue.channelId}
            originCreatedAt={issue.createdAt}
            originPubkey={issue.author}
            query={entityDiscussionQuery(issue.id)}
            testId="issue-discussed-in"
          />
          <ProjectIssueCommentTimeline
            comments={issue.comments}
            key={issue.id}
            profiles={profiles}
          />
        </div>
      </ProjectDetailSection>
      <div
        className="px-6 pb-6 pt-4"
        data-testid="project-issue-comment-composer"
      >
        <ForumComposer
          className="border border-border/60 bg-background/45"
          disabled={commentMutation.isPending}
          isSending={commentMutation.isPending}
          members={members}
          onSubmit={handleCommentSubmit}
          placeholder="Add a comment…"
          profiles={profiles}
        />
      </div>
    </div>
  );
}

export function ProjectIssuesPanel({
  error,
  isLoading,
  issueItems,
  onSelectedIssueIdChange,
  profiles,
  project,
  selectedIssueId,
}: {
  error?: unknown;
  isLoading?: boolean;
  issueItems?: ProjectIssuePanelItem[];
  onSelectedIssueIdChange: (id: string | null) => void;
  profiles?: UserProfileLookup;
  project: Project;
  selectedIssueId: string | null;
}) {
  const issuesQuery = useProjectIssuesQuery(
    issueItems === undefined ? project : null,
  );
  const resolvedItems =
    issueItems ?? (issuesQuery.data ?? []).map((issue) => ({ issue, project }));
  const selectedItem =
    resolvedItems.find(({ issue }) => issue.id === selectedIssueId) ?? null;
  const loading = isLoading ?? issuesQuery.isLoading;
  const loadError = error ?? issuesQuery.error;

  if (loading) {
    return <BuzzLoadingState label="Loading tasks" />;
  }

  if (resolvedItems.length === 0) {
    return (
      <ProjectPanelState
        description={
          loadError
            ? "Refresh the project and try again."
            : issueItems
              ? "Tasks created for this project's repositories will appear here."
              : "Tasks created for this repository will appear here."
        }
        error={Boolean(loadError)}
        title={loadError ? "Could not load tasks" : "No tasks yet"}
      />
    );
  }

  if (selectedItem) {
    return (
      <ProjectIssueDetail
        issue={selectedItem.issue}
        profiles={profiles}
        project={selectedItem.project}
      />
    );
  }

  const groups = ISSUE_STATUS_ORDER.map((status) => ({
    items: resolvedItems.filter(({ issue }) => issue.status === status),
    status,
  })).filter((group) => group.items.length > 0);
  const rangeItems = resolvedItems.map(({ issue, project: itemProject }) =>
    issueSelectionItem(itemProject, issue),
  );

  return (
    <div>
      {groups.map(({ items, status }) => {
        const visual = issueStatusVisual(status);
        return (
          <ProjectWorkItemGroup
            count={items.length}
            icon={
              <ProjectStatusProgressIcon
                className={`h-4 w-4 ${visual.className}`}
                state={visual.progress}
              />
            }
            items={items.map(({ issue, project: itemProject }) =>
              issueSelectionItem(itemProject, issue),
            )}
            key={status}
            label={status}
          >
            {items.map(({ issue, project: itemProject }) => (
              <IssueRow
                issue={issue}
                key={issue.id}
                onOpen={() => onSelectedIssueIdChange(issue.id)}
                profiles={profiles}
                project={itemProject}
                rangeItems={rangeItems}
              />
            ))}
          </ProjectWorkItemGroup>
        );
      })}
    </div>
  );
}
