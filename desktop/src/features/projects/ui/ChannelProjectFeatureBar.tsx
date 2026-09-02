import {
  ArrowLeft,
  GitBranch,
  GitPullRequest,
  Hash,
  ListTodo,
  MessagesSquare,
  Plus,
} from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useCreateChannelMutation } from "@/features/channels/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

import { useCreateProjectIssueMutation } from "../issueMutations";
import { useChannelProjectFeatures } from "../useChannelProjectFeatures";
import { CreateProjectWorkItemDialog } from "./CreateProjectWorkItemDialog";
import { ProjectIssuesPanel } from "./ProjectIssuesPanel";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";

type OpenTool = "tasks" | "breakouts" | "repositories" | null;

export function ChannelProjectFeatureBar({
  channel,
  currentPubkey,
}: {
  channel: Channel;
  currentPubkey?: string;
}) {
  const { activeCommunity } = useCommunities();
  const { goChannel, goProject } = useAppNavigation();
  const context = useChannelProjectFeatures({
    channel,
    currentPubkey,
    relayUrl: activeCommunity?.relayUrl,
  });
  const createChannelMutation = useCreateChannelMutation();
  const createTaskMutation = useCreateProjectIssueMutation(
    context.primaryRepository,
  );
  const [openTool, setOpenTool] = React.useState<OpenTool>(null);
  const [createTaskOpen, setCreateTaskOpen] = React.useState(false);
  const [createChannelOpen, setCreateChannelOpen] = React.useState(false);
  const [selectedIssueId, setSelectedIssueId] = React.useState<string | null>(
    null,
  );
  const project = context.project;

  if (
    !project ||
    project.projectChannelId === channel.id ||
    !Object.values(context.enabled).some(Boolean) ||
    channel.channelType === "dm"
  ) {
    return null;
  }

  const breakoutChannels = context.breakoutChannelIds.flatMap((channelId) => {
    const result = context.channels.find(
      (candidate) => candidate.id === channelId,
    );
    return result ? [result] : [];
  });

  return (
    <>
      <nav
        aria-label="Channel features"
        className="pointer-events-auto flex h-10 shrink-0 items-center gap-1 overflow-x-auto border-b border-border/60 px-4"
        data-testid="channel-project-feature-bar"
      >
        {context.enabled.tasks ? (
          <FeatureButton
            icon={ListTodo}
            label="Tasks"
            onClick={() => setOpenTool("tasks")}
            testId="open-channel-tasks"
          />
        ) : null}
        {context.enabled.breakouts ? (
          <FeatureButton
            icon={MessagesSquare}
            label="Breakout channels"
            onClick={() => setOpenTool("breakouts")}
            testId="open-channel-breakouts"
          />
        ) : null}
        {context.enabled.reviews && context.primaryRepository ? (
          <FeatureButton
            icon={GitPullRequest}
            label="Reviews"
            onClick={() =>
              void goProject(project.id, {
                repositoryId: context.primaryRepository?.id,
                tab: "prs",
              })
            }
            testId="open-channel-reviews"
          />
        ) : null}
        {context.enabled.repositories ? (
          <FeatureButton
            icon={GitBranch}
            label="Repositories"
            onClick={() => setOpenTool("repositories")}
            testId="open-channel-repositories"
          />
        ) : null}
      </nav>

      <ToolDialog
        actions={
          <>
            {selectedIssueId ? (
              <Button
                aria-label="Back to tasks"
                onClick={() => setSelectedIssueId(null)}
                size="icon"
                type="button"
                variant="ghost"
              >
                <ArrowLeft className="h-4 w-4" />
              </Button>
            ) : null}
            <Button
              aria-label="Create task"
              data-testid="create-channel-task"
              disabled={!context.primaryRepository}
              onClick={() => setCreateTaskOpen(true)}
              size="icon"
              type="button"
              variant="outline"
            >
              <Plus className="h-4 w-4" />
            </Button>
          </>
        }
        onOpenChange={(open) => {
          setOpenTool(open ? "tasks" : null);
          if (!open) setSelectedIssueId(null);
        }}
        open={openTool === "tasks"}
        testId="channel-tasks-dialog"
        title="Tasks"
      >
        {context.primaryRepository ? (
          <ProjectIssuesPanel
            onSelectedIssueIdChange={setSelectedIssueId}
            project={context.primaryRepository}
            selectedIssueId={selectedIssueId}
          />
        ) : (
          <EmptyState>Tasks are unavailable.</EmptyState>
        )}
      </ToolDialog>

      <CreateProjectWorkItemDialog
        bodyPlaceholder="Add details"
        description={`Create a task for #${channel.name}.`}
        isCreating={createTaskMutation.isPending}
        itemName="issue"
        onCreate={async (input) => {
          await createTaskMutation.mutateAsync(input);
          context.setFeatureEnabled("tasks", true);
        }}
        onOpenChange={setCreateTaskOpen}
        open={createTaskOpen}
        submitDisabled={!context.primaryRepository}
        title="Create task"
        titlePlaceholder="Task title"
      />

      <ToolDialog
        actions={
          <Button
            aria-label="Create breakout channel"
            data-testid="create-breakout-channel"
            onClick={() => setCreateChannelOpen(true)}
            size="icon"
            type="button"
            variant="outline"
          >
            <Plus className="h-4 w-4" />
          </Button>
        }
        onOpenChange={(open) => setOpenTool(open ? "breakouts" : null)}
        open={openTool === "breakouts"}
        testId="channel-breakouts-dialog"
        title="Breakout channels"
      >
        {breakoutChannels.length > 0 ? (
          <div className="divide-y divide-border/60">
            {breakoutChannels.map((breakoutChannel) => (
              <button
                className="flex min-h-12 w-full items-center gap-3 px-5 py-3 text-left text-sm hover:bg-muted/40"
                key={breakoutChannel.id}
                onClick={() => {
                  setOpenTool(null);
                  void goChannel(breakoutChannel.id);
                }}
                type="button"
              >
                <Hash className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span className="truncate">{breakoutChannel.name}</span>
              </button>
            ))}
          </div>
        ) : (
          <EmptyState>No breakout channels yet.</EmptyState>
        )}
      </ToolDialog>

      <CreateChannelDialog
        channelKind={createChannelOpen ? "stream" : null}
        isCreating={createChannelMutation.isPending}
        onCreate={async (input) => {
          const createdChannel = await createChannelMutation.mutateAsync({
            ...input,
            channelType: "stream",
          });
          const section = context.ensureBreakoutSection();
          if (!section) throw new Error("Could not create the channel group.");
          context.channelSections.assignChannel(createdChannel.id, section.id);
          context.setFeatureEnabled("breakouts", true);
        }}
        onOpenChange={setCreateChannelOpen}
      />

      <ToolDialog
        actions={
          context.primaryRepository ? (
            <ProjectRepositoryManagement
              compact
              identityPubkey={currentPubkey}
              onChange={() => context.setFeatureEnabled("repositories", true)}
              project={project}
              projects={context.projects}
              repository={context.primaryRepository}
              showAccessManagement={false}
            />
          ) : null
        }
        onOpenChange={(open) => setOpenTool(open ? "repositories" : null)}
        open={openTool === "repositories"}
        testId="channel-repositories-dialog"
        title="Related repositories"
      >
        {context.relatedRepositories.length > 0 ? (
          <div className="divide-y divide-border/60">
            {context.relatedRepositories.map((repository) => (
              <div
                className="flex min-h-12 items-center gap-3 px-5 py-3 text-sm"
                key={repository.repoAddress}
              >
                <GitBranch className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span className="truncate">{repository.name}</span>
              </div>
            ))}
          </div>
        ) : (
          <EmptyState>No related repositories yet.</EmptyState>
        )}
      </ToolDialog>
    </>
  );
}

function FeatureButton({
  icon: Icon,
  label,
  onClick,
  testId,
}: {
  icon: typeof ListTodo;
  label: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <Button
      className="h-7 shrink-0 gap-1.5 px-2 text-xs"
      data-testid={testId}
      onClick={onClick}
      size="sm"
      type="button"
      variant="ghost"
    >
      <Icon className="h-3.5 w-3.5" />
      {label}
    </Button>
  );
}

function ToolDialog({
  actions,
  children,
  onOpenChange,
  open,
  testId,
  title,
}: {
  actions?: React.ReactNode;
  children: React.ReactNode;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  testId: string;
  title: string;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="flex max-h-[80vh] min-h-72 max-w-3xl flex-col gap-0 overflow-hidden p-0"
        data-testid={testId}
      >
        <DialogHeader className="flex-row items-center justify-between border-b border-border/60 px-5 py-4 pr-14">
          <div>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription className="sr-only">
              {title} for this channel
            </DialogDescription>
          </div>
          <div className="flex items-center gap-1">{actions}</div>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
      </DialogContent>
    </Dialog>
  );
}

function EmptyState({ children }: { children: React.ReactNode }) {
  return <p className="p-5 text-sm text-muted-foreground">{children}</p>;
}
