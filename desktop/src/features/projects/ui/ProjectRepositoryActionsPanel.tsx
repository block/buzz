import {
  CheckCircle2,
  CircleDot,
  DownloadCloud,
  ExternalLink,
  FileCode2,
  FolderOpen,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  Loader2,
  MessageCircle,
  Plus,
  RefreshCw,
  SquareTerminal,
  UploadCloud,
  Users,
} from "lucide-react";
import type * as React from "react";

import type {
  Project,
  ProjectIssue,
  ProjectPullRequest,
  ProjectRepoContributor,
  ProjectRepoFile,
  ProjectRepoSnapshot,
  Repository,
} from "@/features/projects/hooks";
import { projectExternalRefUrl } from "@/features/projects/lib/projectExternalUrl";
import {
  type ProjectSelectionItem,
  projectSelectionPresentation,
} from "@/features/projects/lib/projectSelection";
import { useProjectSelection } from "@/features/projects/lib/useProjectSelection";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  RepoSourceDropdown,
  type RepoSourceHeaderControls,
  RepositoryBranchDropdown,
} from "./ProjectRepositorySource";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";
import { ProjectWorkItemContextActions } from "./ProjectWorkItemContextActions";
import { ProjectWorkItemCommunicationActions } from "./ProjectWorkItemCommunicationActions";
import { ProjectWorkItemContextDetails } from "./ProjectWorkItemContextDetails";
import { ProjectsSelectionCountMenu } from "./ProjectsSelectionCountMenu";
import {
  projectReviewActivity,
  projectRightPanelScope,
  projectTaskActivity,
} from "./projectRightPanelContext";
import { PROJECT_CONTEXT_ACTION_BUTTON_CLASS } from "./projectContextActionStyles";

type ProjectRepositoryActionsPanelProps = {
  activeTab: string;
  canResetWidth: boolean;
  contributors: ProjectRepoContributor[];
  contextItem?: ProjectSelectionItem | null;
  createIssuePending: boolean;
  detached?: boolean;
  files: ProjectRepoFile[];
  identityPubkey?: string;
  issues: ProjectIssue[];
  onChatWithAgent: (items: ProjectSelectionItem[]) => void;
  onCreateTask: () => void;
  onCreatePullRequest?: () => void;
  onOpenLocalRepository: () => void;
  onOpenTerminal: () => void;
  onRepositoryChange: (repositoryId: string) => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  profiles?: UserProfileLookup;
  pullRequests: ProjectPullRequest[];
  project: Project;
  projects: Project[];
  repository: Repository;
  selectedIssue?: ProjectIssue | null;
  selectedPullRequest?: ProjectPullRequest | null;
  snapshot: ProjectRepoSnapshot | null | undefined;
  sourceControls: RepoSourceHeaderControls;
  terminalTitle?: string;
  widthPx: number;
};

function RepositoryPanelSection({
  children,
  divided = false,
  title,
}: {
  children: React.ReactNode;
  divided?: boolean;
  title?: string;
}) {
  return (
    <section className={divided ? "!mt-2 space-y-0.5 pt-2" : "space-y-0.5"}>
      {title ? (
        <h3 className="flex h-7 min-w-0 items-center truncate text-sm font-normal text-muted-foreground/70">
          {title}
        </h3>
      ) : null}
      {children}
    </section>
  );
}

function RepositoryPeople({
  people,
  profiles,
}: {
  people: string[];
  profiles?: UserProfileLookup;
}) {
  return (
    <div
      className="flex -space-x-1.5 pb-1 pt-0"
      data-testid="project-repository-people"
    >
      {people.slice(0, 18).map((person) => {
        const profile = profiles?.[person];
        const label =
          profile?.displayName?.trim() ||
          profile?.nip05Handle?.trim() ||
          person;
        return (
          <Tooltip key={person}>
            <TooltipTrigger asChild>
              <span
                className="relative inline-flex rounded-full ring-2 ring-background"
                data-testid="project-repository-person"
              >
                <UserAvatar
                  accent={profile?.isAgent === true}
                  avatarUrl={profile?.avatarUrl ?? null}
                  displayName={label}
                  size="sm"
                />
              </span>
            </TooltipTrigger>
            <TooltipContent data-testid="project-repository-person-tooltip">
              {label}
            </TooltipContent>
          </Tooltip>
        );
      })}
    </div>
  );
}

function RepositoryActionButton({
  children,
  disabled,
  onClick,
  title,
}: {
  children: React.ReactNode;
  disabled?: boolean;
  onClick: () => void;
  title?: string;
}) {
  return (
    <Button
      className={PROJECT_CONTEXT_ACTION_BUTTON_CLASS}
      disabled={disabled}
      onClick={onClick}
      size="sm"
      title={title}
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

export function ProjectRepositoryActionsPanel({
  activeTab,
  canResetWidth,
  contributors,
  contextItem,
  createIssuePending,
  detached = false,
  files,
  identityPubkey,
  issues,
  onChatWithAgent,
  onCreateTask,
  onCreatePullRequest,
  onOpenLocalRepository,
  onOpenTerminal,
  onRepositoryChange,
  onResetWidth,
  onResizeStart,
  profiles,
  pullRequests,
  project,
  projects,
  repository,
  selectedIssue,
  selectedPullRequest,
  snapshot,
  sourceControls,
  terminalTitle,
  widthPx,
}: ProjectRepositoryActionsPanelProps) {
  const contextPeople = selectedIssue
    ? [selectedIssue.author, ...selectedIssue.assignees]
    : selectedPullRequest
      ? [
          selectedPullRequest.author,
          ...selectedPullRequest.reviewers,
          ...selectedPullRequest.approvals.map(({ author }) => author),
          ...selectedPullRequest.changeRequests.map(({ author }) => author),
        ]
      : [repository.owner, ...repository.contributors];
  const people = [
    ...new Set(contextPeople.filter(Boolean).map(normalizePubkey)),
  ];
  const selection = useProjectSelection();
  const selectionPresentation = projectSelectionPresentation(
    selection?.items ?? [],
  );
  const latestCommit = snapshot?.latestCommit ?? null;
  const scope = projectRightPanelScope(activeTab);
  const branchScoped = scope === "branch";
  const taskActivity = projectTaskActivity(issues);
  const reviewActivity = projectReviewActivity(pullRequests);
  const cloneAction = sourceControls.localDisabled
    ? sourceControls.onCloneLocal
    : undefined;
  const fetchAction =
    branchScoped &&
    (sourceControls.source === "local" ||
      sourceControls.remoteKind !== "external")
      ? sourceControls.onFetch
      : undefined;
  const pullAction =
    branchScoped && sourceControls.canPull ? sourceControls.onPull : undefined;
  const pushAction =
    branchScoped && sourceControls.canPush ? sourceControls.onPush : undefined;
  const externalOpenUrl = projectExternalRefUrl(
    sourceControls.externalUrl,
    branchScoped ? (sourceControls.selectedTag ?? sourceControls.branch) : null,
  );

  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      constrainToAvailableSpace={!detached}
      detached={detached}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId="project-repository-actions-panel"
      widthPx={widthPx}
    >
      <div
        className={
          detached
            ? "relative z-30 min-h-0 flex-1 overflow-y-auto"
            : "relative z-30 min-h-0 flex-1 overflow-y-auto p-3"
        }
      >
        <div
          className={`rounded-2xl ${detached ? "bg-background" : "border border-border/60 bg-muted/30"}`}
          data-testid="project-context-card"
        >
          <div
            className={`flex min-w-0 items-center justify-between gap-2 px-4 pt-3 ${
              selectionPresentation ? "pb-3" : ""
            }`}
          >
            {selectionPresentation && selection ? (
              <ProjectsSelectionCountMenu
                onChatWithAgent={onChatWithAgent}
                onCreatePullRequest={onCreatePullRequest}
                presentation={selectionPresentation}
                selectionItems={selection.items}
              />
            ) : (
              <>
                <h2 className="min-w-0 truncate text-sm font-normal text-muted-foreground/70">
                  {selectedIssue?.title ??
                    selectedPullRequest?.title ??
                    repository.name}
                </h2>
                {branchScoped ? (
                  <div
                    className="flex shrink-0 items-center"
                    data-testid="project-repository-management-actions"
                  >
                    <ProjectRepositoryManagement
                      compact
                      identityPubkey={identityPubkey}
                      onChange={onRepositoryChange}
                      project={project}
                      projects={projects}
                      repository={repository}
                    />
                  </div>
                ) : null}
              </>
            )}
          </div>
          <div
            className={`space-y-0.5 px-4 pb-3 pt-2 ${
              selectionPresentation ? "hidden" : ""
            }`}
          >
            <ProjectWorkItemContextActions
              issue={selectedIssue}
              profiles={profiles}
              pullRequest={selectedPullRequest}
              repository={repository}
            />
            {contextItem ? (
              <ProjectWorkItemCommunicationActions
                item={contextItem}
                onChatWithAgent={onChatWithAgent}
              />
            ) : null}
            {branchScoped ? (
              <RepositoryPanelSection>
                <div className="grid gap-0.5 [&_button]:-mx-2 [&_button]:h-7 [&_button]:w-[calc(100%+1rem)] [&_button]:max-w-none [&_button]:justify-start [&_button]:gap-3 [&_button]:rounded-md [&_button]:border-0 [&_button]:bg-transparent [&_button]:px-2 [&_button]:text-left [&_button]:text-sm [&_button]:font-normal [&_button]:shadow-none [&_button]:hover:bg-muted/70 [&_button>svg:last-child]:ml-auto">
                  <RepoSourceDropdown controls={sourceControls} />
                  <RepositoryBranchDropdown
                    branch={sourceControls.branch}
                    branchOptions={sourceControls.branchOptions}
                    createBranchDisabled={sourceControls.createBranchDisabled}
                    createBranchTitle={sourceControls.createBranchTitle}
                    deleteBranchDisabled={sourceControls.deleteBranchDisabled}
                    deleteBranchTitle={sourceControls.deleteBranchTitle}
                    onBranchChange={sourceControls.onBranchChange}
                    onCreateBranch={sourceControls.onCreateBranch}
                    onDeleteBranch={sourceControls.onDeleteBranch}
                    onTagChange={sourceControls.onTagChange}
                    selectedTag={sourceControls.selectedTag}
                    tagOptions={sourceControls.tagOptions}
                  />
                </div>
              </RepositoryPanelSection>
            ) : activeTab === "issues" && !selectionPresentation ? (
              <RepositoryPanelSection>
                <RepositoryActionButton
                  disabled={createIssuePending}
                  onClick={onCreateTask}
                >
                  <Plus className="h-3.5 w-3.5" />
                  Create task
                </RepositoryActionButton>
              </RepositoryPanelSection>
            ) : null}

            {branchScoped ? (
              <RepositoryPanelSection>
                <div className="grid gap-0.5">
                  {cloneAction ? (
                    <RepositoryActionButton
                      disabled={sourceControls.clonePending}
                      onClick={cloneAction}
                    >
                      {sourceControls.clonePending ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <DownloadCloud className="h-3.5 w-3.5" />
                      )}
                      {sourceControls.clonePending ? "Cloning…" : "Clone"}
                    </RepositoryActionButton>
                  ) : null}
                  {sourceControls.remoteUnavailableReason === "access" &&
                  sourceControls.onAskForAccess ? (
                    <RepositoryActionButton
                      onClick={sourceControls.onAskForAccess}
                      title="Open project chat to ask for repository access"
                    >
                      <MessageCircle className="h-3.5 w-3.5" />
                      Ask for access
                    </RepositoryActionButton>
                  ) : null}
                  {fetchAction ? (
                    <RepositoryActionButton
                      disabled={sourceControls.fetchPending}
                      onClick={fetchAction}
                      title={sourceControls.fetchTitle}
                    >
                      {sourceControls.fetchPending ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <RefreshCw className="h-3.5 w-3.5" />
                      )}
                      Fetch
                    </RepositoryActionButton>
                  ) : null}
                  {pullAction ? (
                    <RepositoryActionButton
                      disabled={sourceControls.pullDisabled}
                      onClick={pullAction}
                      title={sourceControls.pullTitle}
                    >
                      {sourceControls.pullPending ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <DownloadCloud className="h-3.5 w-3.5" />
                      )}
                      Pull
                      {sourceControls.behindCount
                        ? ` ${sourceControls.behindCount}`
                        : ""}
                    </RepositoryActionButton>
                  ) : null}
                  {pushAction ? (
                    <RepositoryActionButton
                      disabled={sourceControls.pushDisabled}
                      onClick={pushAction}
                      title={sourceControls.pushTitle}
                    >
                      {sourceControls.pushPending ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <UploadCloud className="h-3.5 w-3.5" />
                      )}
                      Push
                      {sourceControls.aheadCount
                        ? ` ${sourceControls.aheadCount}`
                        : ""}
                    </RepositoryActionButton>
                  ) : null}
                  <RepositoryActionButton
                    onClick={onOpenTerminal}
                    title={terminalTitle ?? "Open terminal"}
                  >
                    <SquareTerminal className="h-3.5 w-3.5" />
                    Terminal
                  </RepositoryActionButton>
                  {sourceControls.source === "local" ? (
                    <RepositoryActionButton
                      onClick={onOpenLocalRepository}
                      title="Open local repository folder"
                    >
                      <FolderOpen className="h-3.5 w-3.5" />
                      Open
                    </RepositoryActionButton>
                  ) : externalOpenUrl ? (
                    <Button
                      asChild
                      className="-mx-2 h-7 w-[calc(100%+1rem)] justify-start gap-3 rounded-md px-2 text-left text-sm font-normal hover:bg-muted/70 [&_svg]:h-4 [&_svg]:w-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground"
                      size="sm"
                      variant="ghost"
                    >
                      <a
                        href={externalOpenUrl}
                        rel="noreferrer"
                        target="_blank"
                      >
                        <ExternalLink className="h-3.5 w-3.5" />
                        Open
                      </a>
                    </Button>
                  ) : null}
                </div>
              </RepositoryPanelSection>
            ) : null}

            {branchScoped ? (
              <RepositoryPanelSection divided title="Details">
                <RepositoryPeople people={people} profiles={profiles} />
                <dl className="text-sm [&>div]:h-7 [&_dd]:ml-auto [&_dd]:tabular-nums [&_dt]:gap-3 [&_dt]:text-foreground [&_dt_svg]:h-4 [&_dt_svg]:w-4 [&_dt_svg]:shrink-0 [&_dt_svg]:text-muted-foreground">
                  <div className="flex items-center justify-between gap-3">
                    <dt className="flex items-center gap-3 text-muted-foreground">
                      <GitCommitHorizontal className="h-3.5 w-3.5" />
                      Latest
                    </dt>
                    <dd className="font-mono text-xs text-foreground">
                      {latestCommit ? latestCommit.hash.slice(0, 7) : "—"}
                    </dd>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <dt className="flex items-center gap-3 text-muted-foreground">
                      <FileCode2 className="h-3.5 w-3.5" />
                      Files
                    </dt>
                    <dd
                      className="font-medium text-foreground"
                      data-testid="project-repository-file-count"
                    >
                      {snapshot ? files.length : "—"}
                    </dd>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <dt className="flex items-center gap-3 text-muted-foreground">
                      <Users className="h-3.5 w-3.5" />
                      Contributors
                    </dt>
                    <dd className="font-medium text-foreground">
                      {snapshot ? contributors.length : "—"}
                    </dd>
                  </div>
                </dl>
              </RepositoryPanelSection>
            ) : (
              <>
                <RepositoryPanelSection
                  divided={activeTab === "issues"}
                  title={
                    selectedIssue || selectedPullRequest
                      ? "People"
                      : activeTab === "issues"
                        ? "Details"
                        : undefined
                  }
                >
                  <RepositoryPeople people={people} profiles={profiles} />
                </RepositoryPanelSection>
                <RepositoryPanelSection
                  title={
                    selectedIssue
                      ? "Task details"
                      : selectedPullRequest
                        ? "Review details"
                        : activeTab === "issues"
                          ? undefined
                          : activeTab === "prs"
                            ? "Review activity"
                            : "Repository activity"
                  }
                >
                  <dl className="text-sm [&>div]:h-7 [&_dd]:ml-auto [&_dd]:tabular-nums [&_dt]:gap-3 [&_dt]:text-foreground [&_dt_svg]:h-4 [&_dt_svg]:w-4 [&_dt_svg]:shrink-0 [&_dt_svg]:text-muted-foreground">
                    <ProjectWorkItemContextDetails
                      issue={selectedIssue}
                      pullRequest={selectedPullRequest}
                    />
                    {!selectedIssue &&
                    !selectedPullRequest &&
                    activeTab !== "prs" ? (
                      <>
                        <div className="flex items-center justify-between gap-3">
                          <dt className="flex items-center gap-3 text-muted-foreground">
                            <CircleDot className="h-3.5 w-3.5" />
                            {activeTab === "issues" ? "Tasks" : "Active tasks"}
                          </dt>
                          <dd className="font-medium text-foreground">
                            {activeTab === "issues"
                              ? taskActivity.total
                              : taskActivity.active}
                          </dd>
                        </div>
                        {activeTab === "issues" ? (
                          <>
                            <div className="flex items-center justify-between gap-3">
                              <dt className="flex items-center gap-3 text-muted-foreground">
                                <CircleDot className="h-3.5 w-3.5" />
                                Active
                              </dt>
                              <dd className="font-medium text-foreground">
                                {taskActivity.active}
                              </dd>
                            </div>
                            <div className="flex items-center justify-between gap-3">
                              <dt className="flex items-center gap-3 text-muted-foreground">
                                <CheckCircle2 className="h-3.5 w-3.5" />
                                Completed
                              </dt>
                              <dd className="font-medium text-foreground">
                                {taskActivity.completed}
                              </dd>
                            </div>
                          </>
                        ) : null}
                      </>
                    ) : null}
                    {!selectedIssue &&
                    !selectedPullRequest &&
                    activeTab !== "issues" ? (
                      <>
                        <div className="flex items-center justify-between gap-3">
                          <dt className="flex items-center gap-3 text-muted-foreground">
                            <GitPullRequest className="h-3.5 w-3.5" />
                            {activeTab === "prs" ? "Reviews" : "Open reviews"}
                          </dt>
                          <dd className="font-medium text-foreground">
                            {activeTab === "prs"
                              ? reviewActivity.total
                              : reviewActivity.open}
                          </dd>
                        </div>
                        {activeTab === "prs" ? (
                          <>
                            <div className="flex items-center justify-between gap-3">
                              <dt className="flex items-center gap-3 text-muted-foreground">
                                <GitPullRequest className="h-3.5 w-3.5" />
                                Open
                              </dt>
                              <dd className="font-medium text-foreground">
                                {reviewActivity.open}
                              </dd>
                            </div>
                            <div className="flex items-center justify-between gap-3">
                              <dt className="flex items-center gap-3 text-muted-foreground">
                                <GitMerge className="h-3.5 w-3.5" />
                                Merged
                              </dt>
                              <dd className="font-medium text-foreground">
                                {reviewActivity.merged}
                              </dd>
                            </div>
                          </>
                        ) : null}
                      </>
                    ) : null}
                  </dl>
                </RepositoryPanelSection>
              </>
            )}
          </div>
        </div>
      </div>
    </RightAuxiliaryPane>
  );
}
