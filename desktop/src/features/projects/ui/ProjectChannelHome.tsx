import { useQueries } from "@tanstack/react-query";
import { useSearch } from "@tanstack/react-router";
import { ArrowLeft, Maximize2, Plus } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { ChannelScreenLoadingFallback } from "@/features/channels/ui/ChannelScreenLoadingFallback";
import { ChannelViewOverrideProvider } from "@/features/channels/ui/ChannelViewOverrideContext";
import { useCommunities } from "@/features/communities/useCommunities";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import {
  type Project,
  useProjectPullRequestsQuery,
} from "@/features/projects/hooks";
import {
  projectHomeWorkspaceSheetExpandTab,
  projectHomeWorkspaceSheetTitle,
} from "@/features/projects/lib/projectHomeWorkspaceSheet";
import { ProjectSelectionProvider } from "@/features/projects/lib/useProjectSelection";
import { useChannelProjectFeatures } from "@/features/projects/useChannelProjectFeatures";
import { useHealProjectHomeRepositories } from "@/features/projects/useHealProjectHomeRepositories";
import { useLiveProjectWorkItems } from "@/features/projects/useLiveProjectWorkItems";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { getAvatarSnapshotUrl } from "@/shared/lib/animatedAvatar";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { ProjectChannelResourcesView } from "./ProjectChannelResourcesView";
import {
  fetchCanvasAvatarDataUrl,
  selectAvatarsWithinBudget,
  toCanvasAvatarUploads,
} from "./project-canvas/canvasAvatars";
import { ProjectCanvasSurface } from "./project-canvas/ProjectCanvasSurface";
import type { ProjectCanvasOpenTarget } from "./project-canvas/projectCanvasBroker";
import {
  publishProjectCanvasAvatars,
  type ProjectCanvasPackageRequest,
} from "./project-canvas/projectCanvasCommands";
import type { ProjectCanvasSnapshots } from "./project-canvas/projectCanvasProtocol";
import { useProjectCanvasBroker } from "./project-canvas/useProjectCanvasBroker";
import {
  ProjectChannelTabs,
  projectChannelViewEnabled,
  type ProjectChannelView,
} from "./ProjectChannelTabs";
import { ProjectDetailChrome } from "./ProjectDetailChrome";
import {
  ProjectHomeWorkspaceSheet,
  type ProjectHomeWorkspaceCreateAction,
  type ProjectHomeWorkspaceDetail,
} from "./ProjectHomeWorkspaceSheet";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";

const EMPTY_TARGET_MESSAGE_EVENTS: RelayEvent[] = [];
const MAX_CANVAS_CHANNELS = 64;
const MAX_CANVAS_MEMBER_PROFILES = 128;
const MAX_CANVAS_PEOPLE_PER_CHANNEL = 5;
const MAX_CANVAS_REPOSITORIES = 64;
const MAX_CANVAS_REVIEWS = 32;
/** Avatars inlined into the channel snapshot, bounded by the RPC ceiling. */
const MAX_CANVAS_AVATARS = 8;
/**
 * Avatars fetched and published for frames to load by pubkey. Published
 * pictures cost nothing in the snapshot, so this is bounded by fetch volume
 * rather than by message size, and it stays inside the backend's
 * per-project store.
 */
const MAX_CANVAS_PUBLISHED_AVATARS = 32;

function boundedCanvasText(value: string, maxLength: number): string {
  return value.slice(0, maxLength);
}

const ChannelScreenView = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelScreen");
  return { default: module.ChannelScreen };
});

function ignoreForumPost() {}
function ignoreForumPostSelect() {}

export function ProjectChannelHome({
  allowRepositoryHealing,
  autoSendDraftKey,
  channel,
  project,
  projects,
  targetMessageEvents = EMPTY_TARGET_MESSAGE_EVENTS,
  targetMessageId,
}: {
  allowRepositoryHealing: boolean;
  autoSendDraftKey?: string | null;
  channel: Channel;
  project: Project;
  projects: Project[];
  targetMessageEvents?: RelayEvent[];
  targetMessageId?: string | null;
}) {
  const { goChannel, goProfile, goProject } = useAppNavigation();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const channelsQuery = useChannelsQuery();
  const search = useSearch({ strict: false }) as {
    autoSend?: string;
    messageId?: string;
  };
  const [activeView, setActiveView] =
    React.useState<ProjectChannelView>("chat");
  const [addRepositoryOpen, setAddRepositoryOpen] = React.useState(false);
  const [workspaceRepositoryId, setWorkspaceRepositoryId] = React.useState<
    string | null
  >(null);
  const [workspaceCreateAction, setWorkspaceCreateAction] =
    React.useState<ProjectHomeWorkspaceCreateAction | null>(null);
  const [workspaceDetail, setWorkspaceDetail] =
    React.useState<ProjectHomeWorkspaceDetail | null>(null);
  const [canvasWorkspaceSelection, setCanvasWorkspaceSelection] =
    React.useState<{
      issueId?: string;
      pullRequestId?: string;
      seq: number;
    } | null>(null);
  const channelFeatures = useChannelProjectFeatures({
    channel,
    currentPubkey: identityQuery.data?.pubkey,
    relayUrl: activeCommunity?.relayUrl,
  });
  const canvasReviewsQuery = useProjectPullRequestsQuery(
    channelFeatures.primaryRepository,
  );
  const homeChannel =
    channelsQuery.data?.find(
      (candidate) => candidate.id === project.projectChannelId,
    ) ?? null;
  const canvasChannels = React.useMemo(() => {
    const relatedIds = new Set(channelFeatures.breakoutChannelIds);
    return [
      ...(homeChannel ? [homeChannel] : []),
      ...(channelsQuery.data ?? []).filter(
        (candidate) =>
          candidate.id !== homeChannel?.id && relatedIds.has(candidate.id),
      ),
    ].slice(0, MAX_CANVAS_CHANNELS);
  }, [channelFeatures.breakoutChannelIds, channelsQuery.data, homeChannel]);
  const canvasReviewRows = React.useMemo(() => {
    const currentPubkey = identityQuery.data?.pubkey.toLowerCase();
    if (!currentPubkey) return [];
    return (canvasReviewsQuery.data ?? [])
      .filter(
        (review) =>
          review.status === "Open" &&
          review.author.toLowerCase() === currentPubkey,
      )
      .flatMap((review) => {
        const decisions = [
          ...review.approvals.map((decision) => ({
            ...decision,
            status: "Approved" as const,
          })),
          ...review.changeRequests.map((decision) => ({
            ...decision,
            status: "Changes requested" as const,
          })),
        ].sort(
          (left, right) =>
            right.createdAt - left.createdAt || right.id.localeCompare(left.id),
        );
        const latestDecision = decisions[0] ?? null;
        const requestedReviewers = new Set(
          review.reviewers.map((reviewer) => reviewer.toLowerCase()),
        );
        const latestReviewerActivity =
          review.comments
            .filter(
              (comment) =>
                requestedReviewers.has(comment.author.toLowerCase()) &&
                !comment.isTrustedReviewRequest &&
                !comment.reviewDecision &&
                comment.inlineCommentStatus !== "outdated",
            )
            .sort(
              (left, right) =>
                right.createdAt - left.createdAt ||
                right.id.localeCompare(left.id),
            )[0] ?? null;
        const agentPubkey =
          latestDecision?.author.toLowerCase() ??
          latestReviewerActivity?.author.toLowerCase() ??
          [...requestedReviewers][0] ??
          null;
        if (!agentPubkey) return [];
        return [
          {
            agentPubkey,
            branch: review.branchName
              ? boundedCanvasText(review.branchName, 256)
              : null,
            displayId: boundedCanvasText(review.id.slice(0, 8), 8),
            id: boundedCanvasText(review.id, 256),
            status:
              latestDecision?.status ??
              (latestReviewerActivity
                ? ("Reviewing" as const)
                : ("Requested" as const)),
            title: boundedCanvasText(review.title, 256),
          },
        ];
      })
      .slice(0, MAX_CANVAS_REVIEWS);
  }, [canvasReviewsQuery.data, identityQuery.data?.pubkey]);
  const canvasProfilePubkeys = React.useMemo(
    () =>
      [
        ...new Set(
          [
            ...canvasChannels.flatMap((candidate) => candidate.memberPubkeys),
            ...canvasReviewRows.flatMap((review) =>
              review.agentPubkey ? [review.agentPubkey] : [],
            ),
          ].map((pubkey) => pubkey.toLowerCase()),
        ),
      ].slice(0, MAX_CANVAS_MEMBER_PROFILES),
    [canvasChannels, canvasReviewRows],
  );
  const canvasProfilesQuery = useUsersBatchQuery(canvasProfilePubkeys, {
    enabled: canvasProfilePubkeys.length > 0,
  });
  const canvasRequest = React.useMemo<ProjectCanvasPackageRequest | null>(
    () =>
      activeCommunity?.id && project.projectAddress
        ? { communityId: activeCommunity.id, projectId: project.projectAddress }
        : null,
    [activeCommunity?.id, project.projectAddress],
  );
  const canvasAvatarCandidates = React.useMemo(
    () =>
      canvasProfilePubkeys
        .flatMap((pubkey) => {
          const avatarUrl =
            canvasProfilesQuery.data?.profiles[pubkey]?.avatarUrl ?? null;
          const snapshotUrl = getAvatarSnapshotUrl(avatarUrl);
          return snapshotUrl ? [{ pubkey, snapshotUrl }] : [];
        })
        .slice(0, MAX_CANVAS_PUBLISHED_AVATARS),
    [canvasProfilePubkeys, canvasProfilesQuery.data],
  );
  const canvasAvatarQueries = useQueries({
    queries: canvasAvatarCandidates.map(({ pubkey, snapshotUrl }) => ({
      enabled: canvasRequest !== null,
      gcTime: 10 * 60_000,
      queryFn: async () => {
        const dataUrl = await fetchCanvasAvatarDataUrl(snapshotUrl);
        // Publish before resolving. Resolving is what updates the snapshot,
        // and the snapshot update is what re-renders the widget — so the bytes
        // are registered before any frame can request them, whichever order
        // the frame and the fetch happened to complete in.
        if (dataUrl && canvasRequest) {
          await publishProjectCanvasAvatars(
            canvasRequest,
            toCanvasAvatarUploads([{ dataUrl, pubkey }]),
          );
        }
        return dataUrl;
      },
      // Scoped to the project: a cached hit from another project would report
      // an avatar as published that was never published for this one.
      queryKey: [
        "project-canvas-avatar",
        canvasRequest?.communityId ?? null,
        canvasRequest?.projectId ?? null,
        pubkey,
        snapshotUrl,
      ],
      staleTime: 10 * 60_000,
    })),
  });
  const canvasAvatarDataByPubkey = React.useMemo(() => {
    // The channel snapshot ships these to the frame in one RPC message, so the
    // combined ceiling applies here exactly as it does to a people lookup.
    const budgeted = selectAvatarsWithinBudget(
      canvasAvatarCandidates.map((_candidate, index) => {
        const dataUrl = canvasAvatarQueries[index]?.data;
        return dataUrl?.startsWith("data:image/") ? dataUrl : null;
      }),
    );
    const avatars = new Map<string, string>();
    canvasAvatarCandidates.forEach((candidate, index) => {
      const dataUrl = budgeted[index];
      if (dataUrl) avatars.set(candidate.pubkey, dataUrl);
    });
    return avatars;
  }, [canvasAvatarCandidates, canvasAvatarQueries]);
  const canvasSnapshots = React.useMemo<ProjectCanvasSnapshots>(() => {
    const projectSummary = {
      description: boundedCanvasText(project.description, 2_048),
      id: boundedCanvasText(project.projectAddress, 1_024),
      name: boundedCanvasText(project.name, 256),
      owner: boundedCanvasText(project.owner, 64),
      repositories: project.repositories
        .slice(0, MAX_CANVAS_REPOSITORIES)
        .map((repository) => ({
          defaultBranch: boundedCanvasText(repository.defaultBranch, 256),
          description: boundedCanvasText(repository.description, 1_024),
          id: boundedCanvasText(repository.repoAddress, 1_024),
          name: boundedCanvasText(repository.name, 256),
          owner: boundedCanvasText(repository.owner, 64),
          status: boundedCanvasText(repository.status, 64),
        })),
    };

    const emittedCanvasAvatarPubkeys = new Set<string>();
    const visibleChannels = canvasChannels.map((candidate) => ({
      description: boundedCanvasText(candidate.description, 1_024),
      id: boundedCanvasText(candidate.id, 256),
      lastMessageAt: candidate.lastMessageAt,
      memberCount: Math.max(0, candidate.memberCount),
      name: boundedCanvasText(candidate.name, 256),
      people: candidate.memberPubkeys
        .slice(0, MAX_CANVAS_PEOPLE_PER_CHANNEL)
        .map((pubkey) => {
          const normalizedPubkey = pubkey.toLowerCase();
          const profile = canvasProfilesQuery.data?.profiles[normalizedPubkey];
          const displayName = profile?.displayName ?? profile?.name ?? null;
          const avatarDataUrl =
            canvasAvatarDataByPubkey.get(normalizedPubkey) ?? null;
          const includeAvatar =
            avatarDataUrl !== null &&
            emittedCanvasAvatarPubkeys.size < MAX_CANVAS_AVATARS &&
            !emittedCanvasAvatarPubkeys.has(normalizedPubkey);
          if (includeAvatar) {
            emittedCanvasAvatarPubkeys.add(normalizedPubkey);
          }
          return {
            avatarDataUrl: includeAvatar ? avatarDataUrl : null,
            displayName: displayName
              ? boundedCanvasText(displayName, 128)
              : null,
            pubkey: boundedCanvasText(normalizedPubkey, 64),
          };
        }),
      relationship:
        candidate.id === homeChannel?.id
          ? ("home" as const)
          : ("related" as const),
      topic: candidate.topic ? boundedCanvasText(candidate.topic, 512) : null,
    }));
    const channelsState: ProjectCanvasSnapshots["channels"] =
      channelsQuery.isPending
        ? { data: null, status: "loading" }
        : channelsQuery.isError
          ? { data: null, status: "error" }
          : { data: visibleChannels, status: "ready" };

    const reviewsState: ProjectCanvasSnapshots["reviews"] =
      !channelFeatures.primaryRepository
        ? { data: [], status: "ready" }
        : canvasReviewsQuery.isPending || identityQuery.isPending
          ? { data: null, status: "loading" }
          : canvasReviewsQuery.isError
            ? { data: null, status: "error" }
            : {
                data: canvasReviewRows.map((review) => {
                  const profile = review.agentPubkey
                    ? canvasProfilesQuery.data?.profiles[review.agentPubkey]
                    : null;
                  const agentName =
                    profile?.displayName ?? profile?.name ?? null;
                  return {
                    ...review,
                    agentName: agentName
                      ? boundedCanvasText(agentName, 256)
                      : null,
                  };
                }),
                status: "ready",
              };

    return {
      channels: channelsState,
      project: { data: projectSummary, status: "ready" },
      reviews: reviewsState,
    };
  }, [
    canvasReviewsQuery.isError,
    canvasReviewsQuery.isPending,
    canvasAvatarDataByPubkey,
    canvasReviewRows,
    channelFeatures.primaryRepository,
    canvasChannels,
    canvasProfilesQuery.data,
    channelsQuery.isError,
    channelsQuery.isPending,
    homeChannel,
    identityQuery.isPending,
    project,
  ]);
  const waitingForChannel = channelsQuery.isPending && !homeChannel;
  const workspaceTab =
    activeView === "issues"
      ? "issues"
      : activeView === "reviews"
        ? "prs"
        : null;
  const workspaceRepository =
    project.repositories.find(
      (repository) => repository.id === workspaceRepositoryId,
    ) ??
    project.repositories[0] ??
    null;

  const selectView = React.useCallback(
    (view: ProjectChannelView) => {
      if ((view === "issues" || view === "reviews") && !workspaceRepository) {
        setAddRepositoryOpen(true);
        return;
      }
      setWorkspaceCreateAction(null);
      setWorkspaceDetail(null);
      setActiveView(view);
    },
    [workspaceRepository],
  );
  React.useEffect(() => {
    if (!projectChannelViewEnabled(activeView, channelFeatures.enabled)) {
      selectView("chat");
    }
  }, [activeView, channelFeatures.enabled, selectView]);

  useLiveProjectWorkItems(project);
  const handleCanvasOpenTarget = React.useCallback(
    (target: ProjectCanvasOpenTarget) => {
      if (target.type === "channel") {
        void goChannel(target.id);
        return;
      }
      if (target.type === "user") {
        void goProfile(target.pubkey);
        return;
      }
      setCanvasWorkspaceSelection((current) => ({
        ...(target.type === "task"
          ? { issueId: target.id }
          : { pullRequestId: target.id }),
        seq: (current?.seq ?? 0) + 1,
      }));
      selectView(target.type === "task" ? "issues" : "reviews");
    },
    [goChannel, goProfile, selectView],
  );
  const canvasBroker = useProjectCanvasBroker({
    canvasRequest,
    identityPubkey: identityQuery.data?.pubkey,
    issues: {
      data: channelFeatures.issuesQuery.data,
      isError: channelFeatures.issuesQuery.isError,
      isPending: channelFeatures.issuesQuery.isPending,
    },
    onOpenTarget: handleCanvasOpenTarget,
    primaryRepository: channelFeatures.primaryRepository,
    relayUrl: activeCommunity?.relayUrl,
    reviews: {
      data: canvasReviewsQuery.data,
      isError: canvasReviewsQuery.isError,
      isPending: canvasReviewsQuery.isPending,
    },
    snapshots: canvasSnapshots,
  });

  const handleOpenRepository = React.useCallback(
    (repositoryId: string) => {
      void goProject(project.id, { repositoryId });
    },
    [goProject, project.id],
  );
  const handleAddFiles = React.useCallback(() => {
    setAddRepositoryOpen(true);
  }, []);
  const handleFilesAdded = React.useCallback(
    (repositoryId: string) => {
      void goProject(project.id, { repositoryId, tab: "files" });
    },
    [goProject, project.id],
  );
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
    if (!workspaceRepository || !workspaceTab) return;
    void goProject(project.id, {
      repositoryId: workspaceRepository.id,
      ...workspaceDetail?.navigation,
      tab: projectHomeWorkspaceSheetExpandTab(workspaceTab),
    });
  }, [
    goProject,
    project.id,
    workspaceDetail?.navigation,
    workspaceRepository,
    workspaceTab,
  ]);

  const workspaceContent =
    workspaceTab && workspaceRepository ? (
      <div
        className="flex min-h-0 flex-1 flex-col"
        data-testid="project-channel-workspace"
      >
        <div className="flex h-10 shrink-0 items-center justify-between gap-3 border-b border-border/60 px-4">
          <div className="flex min-w-0 items-center gap-2">
            {workspaceDetail ? (
              <Button
                aria-label={workspaceDetail.backLabel}
                className="h-7 w-7 shrink-0"
                onClick={workspaceDetail.onBack}
                size="icon"
                title={workspaceDetail.backLabel}
                type="button"
                variant="ghost"
              >
                <ArrowLeft className="h-4 w-4" />
              </Button>
            ) : null}
            <span className="truncate text-sm font-medium">
              {projectHomeWorkspaceSheetTitle(workspaceTab)}
            </span>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {workspaceCreateAction ? (
              <Tooltip disableHoverableContent>
                <TooltipTrigger asChild>
                  <Button
                    aria-label={workspaceCreateAction.label}
                    className="h-7 w-7"
                    data-testid="project-home-workspace-sheet-create"
                    disabled={workspaceCreateAction.disabled}
                    onClick={workspaceCreateAction.onClick}
                    size="icon"
                    title={
                      workspaceCreateAction.title ?? workspaceCreateAction.label
                    }
                    type="button"
                    variant="ghost"
                  >
                    <Plus className="h-4 w-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{workspaceCreateAction.label}</TooltipContent>
              </Tooltip>
            ) : null}
            <Tooltip disableHoverableContent>
              <TooltipTrigger asChild>
                <Button
                  aria-label={`Open ${projectHomeWorkspaceSheetTitle(workspaceTab)} in repository`}
                  className="h-7 w-7"
                  data-testid="project-home-workspace-sheet-expand"
                  onClick={handleExpandWorkspace}
                  size="icon"
                  title={`Open ${projectHomeWorkspaceSheetTitle(workspaceTab)} in repository`}
                  type="button"
                  variant="ghost"
                >
                  <Maximize2 className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Open in repository</TooltipContent>
            </Tooltip>
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
          <ProjectHomeWorkspaceSheet
            key={`${workspaceTab}:${workspaceRepository.id}:${canvasWorkspaceSelection?.seq ?? 0}`}
            identityPubkey={identityQuery.data?.pubkey}
            initialIssueId={canvasWorkspaceSelection?.issueId ?? null}
            initialPullRequestId={
              canvasWorkspaceSelection?.pullRequestId ?? null
            }
            onCreateActionChange={setWorkspaceCreateAction}
            onDetailChange={setWorkspaceDetail}
            onOpenCommit={handleOpenCommit}
            onRepositoryAdded={handleFilesAdded}
            onSelectRepository={handleWorkspaceRepositoryChange}
            project={project}
            projects={projects}
            repository={workspaceRepository}
            tab={workspaceTab}
          />
        </div>
      </div>
    ) : null;
  const mainContent =
    activeView === "channels" ? (
      <ProjectChannelResourcesView
        channels={channelsQuery.data ?? []}
        identityPubkey={identityQuery.data?.pubkey}
        onOpenChannel={(channelId) => void goChannel(channelId)}
        onOpenRepository={handleOpenRepository}
        onSelectChat={() => selectView("chat")}
        project={project}
        projects={projects}
        relatedChannelIds={channelFeatures.breakoutChannelIds}
        view="channels"
      />
    ) : activeView === "repos" ? (
      <ProjectChannelResourcesView
        channels={channelsQuery.data ?? []}
        identityPubkey={identityQuery.data?.pubkey}
        onOpenChannel={(channelId) => void goChannel(channelId)}
        onOpenRepository={handleOpenRepository}
        onSelectChat={() => selectView("chat")}
        project={project}
        projects={projects}
        view="repos"
      />
    ) : (
      workspaceContent
    );

  return (
    <ProjectSelectionProvider resetKey={`${project.id}:${activeView}`}>
      <div
        className="relative flex min-h-0 min-w-0 flex-1 overflow-hidden bg-background"
        data-project-detail-screen
        data-repository-healing-enabled={allowRepositoryHealing}
        data-testid="project-channel-home"
      >
        <div className="relative flex min-h-0 min-w-60 flex-1 flex-col overflow-hidden">
          <ProjectDetailChrome
            activeTabCrumb={null}
            activeWorkItemCrumb={null}
            onGoProjectHome={() => undefined}
            onGoRootChannel={() => {
              if (project.projectChannelId) {
                void goChannel(project.projectChannelId);
              }
            }}
            project={project}
          />
          {waitingForChannel ? (
            <ViewLoadingFallback kind="channel" />
          ) : homeChannel ? (
            <div
              className="flex min-h-0 min-w-0 flex-1 flex-col"
              data-testid="project-channel-canvas-layout"
            >
              <div
                className="flex min-h-0 min-w-0 flex-1"
                data-testid="project-channel-chat-pane"
              >
                <React.Suspense
                  fallback={
                    <ChannelScreenLoadingFallback isHuddleTranscript={false} />
                  }
                >
                  <ChannelViewOverrideProvider
                    value={{
                      headerNavigation: (
                        <ProjectChannelTabs
                          activeView={activeView}
                          enabledFeatures={channelFeatures.enabled}
                          onSelect={selectView}
                        />
                      ),
                      hideMainColumnBody: activeView === "canvas",
                      isChannelViewActive: activeView === "chat",
                      mainColumnHeader:
                        activeView === "chat" || activeView === "canvas" ? (
                          <ProjectCanvasSurface
                            broker={canvasBroker}
                            communityId={activeCommunity?.id ?? null}
                            full={activeView === "canvas"}
                            onShowFullCanvas={() => selectView("canvas")}
                            projectId={project.projectAddress}
                            projectName={project.name}
                            projectNames={[channel.name, project.name]}
                            snapshots={canvasSnapshots}
                          />
                        ) : null,
                      mainColumnHeaderPlacement:
                        activeView === "chat" ? "right" : "top",
                      mainContent,
                      onSelectChannelView: () => selectView("chat"),
                    }}
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
                      onAddFiles={
                        channelFeatures.enabled.repositories
                          ? handleAddFiles
                          : undefined
                      }
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
                  </ChannelViewOverrideProvider>
                </React.Suspense>
              </div>
            </div>
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
      </div>
    </ProjectSelectionProvider>
  );
}
