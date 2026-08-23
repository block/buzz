import { useSearch } from "@tanstack/react-router";
import { Info } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { ChannelScreenLoadingFallback } from "@/features/channels/ui/ChannelScreenLoadingFallback";
import { useProfileQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  isProjectHomeWorkspaceSheetTab,
  projectHomeWorkspaceSheetTitle,
  type ProjectHomeWorkspaceSheetTab,
} from "@/features/projects/lib/projectHomeWorkspaceSheet";
import { useHealProjectHomeRepositories } from "@/features/projects/useHealProjectHomeRepositories";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { RelayEvent } from "@/shared/api/types";
import type { EntityLinkTab } from "@/shared/lib/entityLink";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
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
import { ProjectHomeWorkspaceSheet } from "./ProjectHomeWorkspaceSheet";
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
  project,
  projects,
}: {
  project: Project;
  projects: Project[];
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
  const summaryWidth = useThreadPanelWidth(undefined, {
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
  const summaryVisible = summaryOpen && !workspaceSheetOpen;

  const openWorkspaceSheet = React.useCallback(
    (tab: ProjectHomeWorkspaceSheetTab, repositoryId?: string) => {
      if (repositoryId) {
        setWorkspaceRepositoryId(repositoryId);
      }
      setWorkspaceSheetTab((current) => (current === tab ? null : tab));
    },
    [],
  );
  const closeWorkspaceSheet = React.useCallback(() => {
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
    setWorkspaceRepositoryId(repositoryId);
    setWorkspaceSheetTab("files");
  }, []);
  const handleToggleFilesSheet = React.useCallback(() => {
    if (!workspaceRepository) {
      handleAddFiles();
      return;
    }
    openWorkspaceSheet("files", workspaceRepository.id);
  }, [handleAddFiles, openWorkspaceSheet, workspaceRepository]);
  useHealProjectHomeRepositories(project, identityQuery.data?.pubkey);
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
  const workspaceSheet =
    workspaceSheetOpen && workspaceSheetTab && workspaceRepository ? (
      <ProjectHomeWorkspaceSheet
        key={`${workspaceSheetTab}:${workspaceRepository.id}`}
        identityPubkey={identityQuery.data?.pubkey}
        onOpenCommit={handleOpenCommit}
        onRepositoryAdded={handleFilesAdded}
        onSelectRepository={setWorkspaceRepositoryId}
        project={project}
        projects={projects}
        repository={workspaceRepository}
        tab={workspaceSheetTab}
      />
    ) : null;

  return (
    <div
      className={cn(
        "relative flex min-h-0 min-w-0 flex-1 overflow-hidden",
        summaryVisible && "bg-sidebar pb-2 pr-2 pt-px",
        summaryVisible && sidebar?.open === false && "pl-2",
      )}
      data-project-context-detached={summaryVisible ? "true" : undefined}
      data-project-detail-screen
      data-testid="project-channel-home"
    >
      <div
        className={cn(
          "relative flex min-h-0 min-w-60 flex-1 flex-col overflow-hidden",
          summaryVisible ? "ml-px rounded-2xl bg-background" : "bg-muted/20",
        )}
      >
        <ProjectDetailChrome
          actions={
            <>
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
                <Info
                  className={cn(
                    "h-4 w-4 transition-opacity duration-200 ease-linear",
                    summaryVisible ? "opacity-100" : "opacity-60",
                  )}
                />
              </ProjectHomeHeaderToggle>
              <ProjectHomeHeaderToggle
                label="Files"
                onClick={handleToggleFilesSheet}
                open={workspaceSheetTab === "files"}
                testId="project-home-codebase-toggle"
              >
                <DrawerPanelIcon
                  className="-scale-x-100"
                  side={workspaceSheetTab === "files" ? "left" : "right"}
                />
              </ProjectHomeHeaderToggle>
            </>
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
              autoSendDraftKey={search.autoSend ?? null}
              currentIdentity={identityQuery.data}
              currentProfile={profileQuery.data}
              idleAuxiliaryPanel={workspaceSheet}
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
              targetMessageEvents={EMPTY_TARGET_MESSAGE_EVENTS}
              targetMessageId={search.messageId ?? null}
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
        open={summaryVisible}
        panelWidthPx={summaryWidth.widthPx}
        resizing={summaryWidth.isResizing}
        testId="project-home-summary-rail"
      >
        {summaryVisible ? (
          <ProjectHomeColumn
            bodyClassName="overflow-y-auto overflow-x-hidden overscroll-contain"
            canResetWidth={summaryWidth.canReset}
            onClose={() => setSummaryOpen(false)}
            onResetWidth={summaryWidth.onResetWidth}
            onResizeStart={summaryWidth.onResizeStart}
            testId="project-home-summary-column"
            title="Overview"
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
  );
}
