import { useSearch } from "@tanstack/react-router";
import { Maximize2, Plus } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { ChannelScreenLoadingFallback } from "@/features/channels/ui/ChannelScreenLoadingFallback";
import { useProfileQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  isProjectHomeWorkspaceSheetTab,
  projectHomeWorkspaceSheetExpandTab,
  projectHomeWorkspaceSheetTitle,
  type ProjectHomeWorkspaceSheetTab,
} from "@/features/projects/lib/projectHomeWorkspaceSheet";
import { ProjectSelectionProvider } from "@/features/projects/lib/useProjectSelection";
import { useHealProjectHomeRepositories } from "@/features/projects/useHealProjectHomeRepositories";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { RelayEvent } from "@/shared/api/types";
import type { EntityLinkTab } from "@/shared/lib/entityLink";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { SIDEBAR_WIDTH_MIN } from "@/shared/layout/sidebarLayout";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";
import { useOptionalSidebar } from "@/shared/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { ProjectContextRail } from "./ProjectContextRail";
import { ProjectDetailChrome } from "./ProjectDetailChrome";
import { ProjectHomeColumn } from "./ProjectHomeColumn";
import { ProjectHomeContextPanel } from "./ProjectHomeContextPanel";
import {
  ProjectHomeWorkspaceSheet,
  type ProjectHomeWorkspaceCreateAction,
  type ProjectHomeWorkspaceDetail,
} from "./ProjectHomeWorkspaceSheet";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";

const EMPTY_TARGET_MESSAGE_EVENTS: RelayEvent[] = [];
const PROJECT_HOME_SUMMARY_WIDTH_KEY =
  "buzz.desktop.project-home-summary-width";

const ChannelScreenView = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelScreen");
  return { default: module.ChannelScreen };
});

function ignoreForumPost() {}
function ignoreForumPostSelect() {}

function ProjectHomeHeaderToggle({
  children,
  label,
  onClick,
  open,
  testId,
}: {
  children: React.ReactNode;
  label: string;
  onClick: () => void;
  open: boolean;
  testId: string;
}) {
  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <Button
          aria-label={open ? `Hide ${label}` : `Show ${label}`}
          aria-pressed={open}
          className="h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
          data-testid={testId}
          onClick={onClick}
          size="icon"
          title={label}
          type="button"
          variant="ghost"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

export function ProjectChannelHome({
  allowRepositoryHealing,
  autoSendDraftKey,
  project,
  projects,
  targetMessageEvents = EMPTY_TARGET_MESSAGE_EVENTS,
  targetMessageId,
}: {
  allowRepositoryHealing: boolean;
  autoSendDraftKey?: string | null;
  project: Project;
  projects: Project[];
  targetMessageEvents?: RelayEvent[];
  targetMessageId?: string | null;
}) {
  const { goChannel, goProject, goProjects } = useAppNavigation();
  const sidebar = useOptionalSidebar();
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const channelsQuery = useChannelsQuery();
  const search = useSearch({ strict: false }) as {
    autoSend?: string;
    messageId?: string;
  };
  const [summaryOpen, setSummaryOpen] = React.useState(true);
  const [addRepositoryOpen, setAddRepositoryOpen] = React.useState(false);
  const [workspaceSheetTab, setWorkspaceSheetTab] =
    React.useState<ProjectHomeWorkspaceSheetTab | null>(null);
  const [workspaceRepositoryId, setWorkspaceRepositoryId] = React.useState<
    string | null
  >(null);
  const [workspaceCreateAction, setWorkspaceCreateAction] =
    React.useState<ProjectHomeWorkspaceCreateAction | null>(null);
  const [workspaceDetail, setWorkspaceDetail] =
    React.useState<ProjectHomeWorkspaceDetail | null>(null);
  const summaryWidth = useThreadPanelWidth(undefined, {
    minWidthPx: SIDEBAR_WIDTH_MIN,
    sessionKey: PROJECT_HOME_SUMMARY_WIDTH_KEY,
  });
  const homeChannel =
    channelsQuery.data?.find(
      (channel) => channel.id === project.projectChannelId,
    ) ?? null;
  const waitingForChannel = channelsQuery.isPending && !homeChannel;
  const workspaceRepository =
    project.repositories.find(
      (repository) => repository.id === workspaceRepositoryId,
    ) ??
    project.repositories[0] ??
    null;
  const workspaceSheetOpen =
    workspaceSheetTab != null && workspaceRepository != null;
  const previousWorkspaceSheetOpenRef = React.useRef(workspaceSheetOpen);
  const workspaceSheetVisibilityChanged =
    previousWorkspaceSheetOpenRef.current !== workspaceSheetOpen;
  React.useEffect(() => {
    previousWorkspaceSheetOpenRef.current = workspaceSheetOpen;
  }, [workspaceSheetOpen]);
  const summaryVisible = summaryOpen && !workspaceSheetOpen;

  const openWorkspaceSheet = React.useCallback(
    (tab: ProjectHomeWorkspaceSheetTab, repositoryId?: string) => {
      if (repositoryId) {
        setWorkspaceRepositoryId(repositoryId);
      }
      setWorkspaceCreateAction(null);
      setWorkspaceDetail(null);
      setWorkspaceSheetTab((current) => (current === tab ? null : tab));
    },
    [],
  );
  const closeWorkspaceSheet = React.useCallback(() => {
    setWorkspaceCreateAction(null);
    setWorkspaceDetail(null);
    setWorkspaceSheetTab(null);
  }, []);
  const handleOpenWorkspace = React.useCallback(
    (repositoryId: string, tab?: EntityLinkTab) => {
      if (!isProjectHomeWorkspaceSheetTab(tab)) {
        void goProject(project.id, { repositoryId, tab });
        return;
      }
      openWorkspaceSheet(tab, repositoryId);
    },
    [goProject, openWorkspaceSheet, project.id],
  );
  const handleOpenRepository = React.useCallback(
    (repositoryId: string) => {
      void goProject(project.id, { repositoryId });
    },
    [goProject, project.id],
  );
  const handleRepositoryChange = React.useCallback(() => {
    void goProject(project.id);
  }, [goProject, project.id]);
  const handleAddFiles = React.useCallback(() => {
    setAddRepositoryOpen(true);
  }, []);
  const handleFilesAdded = React.useCallback((repositoryId: string) => {
    setWorkspaceCreateAction(null);
    setWorkspaceDetail(null);
    setWorkspaceRepositoryId(repositoryId);
    setWorkspaceSheetTab("files");
  }, []);
  const handleWorkspaceRepositoryChange = React.useCallback(
    (repositoryId: string) => {
      setWorkspaceCreateAction(null);
      setWorkspaceDetail(null);
      setWorkspaceRepositoryId(repositoryId);
    },
    [],
  );
  useHealProjectHomeRepositories(
    project,
    allowRepositoryHealing,
    identityQuery.data?.pubkey,
  );
  const handleOpenCommit = React.useCallback(
    (commitHash: string) => {
      if (!workspaceRepository) return;
      void goProject(project.id, {
        commitHash,
        repositoryId: workspaceRepository.id,
        tab: "commits",
      });
    },
    [goProject, project.id, workspaceRepository],
  );
  const handleExpandWorkspace = React.useCallback(() => {
    if (!workspaceRepository || !workspaceSheetTab) return;
    void goProject(project.id, {
      repositoryId: workspaceRepository.id,
      ...workspaceDetail?.navigation,
      tab: projectHomeWorkspaceSheetExpandTab(workspaceSheetTab),
    });
  }, [
    goProject,
    project.id,
    workspaceDetail?.navigation,
    workspaceRepository,
    workspaceSheetTab,
  ]);
  const expandLabel = workspaceSheetTab
    ? `Open ${projectHomeWorkspaceSheetTitle(workspaceSheetTab)} in repository`
    : "Open in repository";
  const workspaceSheet =
    workspaceSheetOpen && workspaceSheetTab && workspaceRepository ? (
      <ProjectHomeWorkspaceSheet
        key={`${workspaceSheetTab}:${workspaceRepository.id}`}
        identityPubkey={identityQuery.data?.pubkey}
        onCreateActionChange={setWorkspaceCreateAction}
        onDetailChange={setWorkspaceDetail}
        onOpenCommit={handleOpenCommit}
        onRepositoryAdded={handleFilesAdded}
        onSelectRepository={handleWorkspaceRepositoryChange}
        project={project}
        projects={projects}
        repository={workspaceRepository}
        tab={workspaceSheetTab}
      />
    ) : null;

  return (
    <ProjectSelectionProvider
      resetKey={`${project.id}:${workspaceSheetTab ?? "home"}`}
    >
      <div
        className={cn(
          "relative flex min-h-0 min-w-0 flex-1 overflow-hidden",
          summaryVisible && "bg-sidebar",
          summaryVisible && sidebar?.open === false && "pl-2",
        )}
        data-project-context-detached={summaryVisible ? "true" : undefined}
        data-project-detail-screen
        data-repository-healing-enabled={allowRepositoryHealing}
        data-testid="project-channel-home"
      >
        <div
          className={cn(
            "relative flex min-h-0 min-w-60 flex-1 flex-col overflow-hidden",
            summaryVisible
              ? "mb-2 ml-px mt-px rounded-2xl bg-background"
              : "bg-muted/20",
          )}
        >
          <ProjectDetailChrome
            actions={
              <ProjectHomeHeaderToggle
                label="Overview"
                onClick={() => {
                  if (workspaceSheetOpen) {
                    closeWorkspaceSheet();
                    return;
                  }
                  setSummaryOpen((open) => !open);
                }}
                open={summaryVisible}
                testId="project-home-drawer-toggle"
              >
                <DrawerPanelIcon
                  className="-scale-x-100"
                  side={summaryVisible ? "left" : "right"}
                />
              </ProjectHomeHeaderToggle>
            }
            activeTabCrumb={null}
            activeWorkItemCrumb={null}
            onGoProjectHome={() => undefined}
            onGoProjects={() => {
              void goProjects();
            }}
            project={project}
          />
          {waitingForChannel ? (
            <ViewLoadingFallback kind="channel" />
          ) : homeChannel ? (
            <React.Suspense
              fallback={
                <ChannelScreenLoadingFallback isHuddleTranscript={false} />
              }
            >
              <ChannelScreenView
                activeChannel={homeChannel}
                autoSendDraftKey={
                  autoSendDraftKey === undefined
                    ? (search.autoSend ?? null)
                    : autoSendDraftKey
                }
                currentIdentity={identityQuery.data}
                currentProfile={profileQuery.data}
                idleAuxiliaryPanel={workspaceSheet}
                idleAuxiliaryHeaderActions={{
                  actions: (
                    <>
                      {workspaceCreateAction ? (
                        <Tooltip disableHoverableContent>
                          <TooltipTrigger asChild>
                            <Button
                              aria-label={workspaceCreateAction.label}
                              className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
                              data-testid="project-home-workspace-sheet-create"
                              disabled={workspaceCreateAction.disabled}
                              onClick={workspaceCreateAction.onClick}
                              size="icon"
                              title={
                                workspaceCreateAction.title ??
                                workspaceCreateAction.label
                              }
                              type="button"
                              variant="ghost"
                            >
                              <Plus className="h-4 w-4" />
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>
                            {workspaceCreateAction.label}
                          </TooltipContent>
                        </Tooltip>
                      ) : null}
                      <Tooltip disableHoverableContent>
                        <TooltipTrigger asChild>
                          <Button
                            aria-label={expandLabel}
                            className="shrink-0"
                            data-testid="project-home-workspace-sheet-expand"
                            onClick={handleExpandWorkspace}
                            size="icon"
                            title={expandLabel}
                            type="button"
                            variant="ghost"
                          >
                            <Maximize2 />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>{expandLabel}</TooltipContent>
                      </Tooltip>
                    </>
                  ),
                  backLabel: workspaceDetail?.backLabel,
                  onBack: workspaceDetail?.onBack,
                }}
                idleAuxiliaryOverridesThread={workspaceSheetOpen}
                idleAuxiliaryTitle={
                  workspaceSheetTab
                    ? projectHomeWorkspaceSheetTitle(workspaceSheetTab)
                    : ""
                }
                onAddFiles={handleAddFiles}
                onCloseIdleAuxiliaryPanel={closeWorkspaceSheet}
                onCloseForumPost={ignoreForumPost}
                onSelectForumPost={ignoreForumPostSelect}
                selectedForumPostId={null}
                targetForumReplyId={null}
                targetMessageEvents={targetMessageEvents}
                targetMessageId={
                  targetMessageId === undefined
                    ? (search.messageId ?? null)
                    : targetMessageId
                }
              />
            </React.Suspense>
          ) : (
            <div className="flex min-h-0 flex-1 items-center justify-center px-6 py-8">
              <p className="text-sm text-muted-foreground">
                This project's channel could not be found.
              </p>
            </div>
          )}
        </div>
        <ProjectRepositoryManagement
          createOpen={addRepositoryOpen}
          hideTriggers
          identityPubkey={identityQuery.data?.pubkey}
          onChange={handleFilesAdded}
          onCreateOpenChange={setAddRepositoryOpen}
          project={project}
          projects={projects}
        />
        <ProjectContextRail
          animateWidth={!workspaceSheetVisibilityChanged}
          open={summaryVisible}
          panelWidthPx={summaryWidth.widthPx}
          resizing={summaryWidth.isResizing}
          rounded={false}
          testId="project-home-summary-rail"
        >
          {summaryVisible ? (
            <ProjectHomeColumn
              bodyClassName="overflow-y-auto overflow-x-hidden overscroll-contain"
              canResetWidth={summaryWidth.canReset}
              onResetWidth={summaryWidth.onResetWidth}
              onResizeStart={summaryWidth.onResizeStart}
              testId="project-home-summary-column"
              widthPx={summaryWidth.widthPx}
            >
              <ProjectHomeContextPanel
                activeWorkspaceTab={workspaceSheetTab}
                channel={homeChannel}
                channels={channelsQuery.data ?? []}
                identityPubkey={identityQuery.data?.pubkey}
                onAddRepository={handleAddFiles}
                onOpenChannel={(channelId) => {
                  void goChannel(channelId);
                }}
                onOpenRepository={handleOpenRepository}
                onOpenWorkspace={handleOpenWorkspace}
                onRepositoryChange={handleRepositoryChange}
                project={project}
                projects={projects}
              />
            </ProjectHomeColumn>
          ) : null}
        </ProjectContextRail>
      </div>
    </ProjectSelectionProvider>
  );
}
