import { ArrowLeft, Plus, X } from "lucide-react";
import * as React from "react";

import {
  BwIssueCreatedButNotEnrolledError,
  useCreateProjectBwIssueMutation,
  useProjectBwActivation,
} from "@/features/projects/bwIssueCreation";
import { BW_ISSUE_TEMPLATES } from "@/features/projects/bwIssueTemplates";
import { useCreateProjectIssueMutation } from "@/features/projects/issueMutations";
import { useProjectsQuery } from "@/features/projects/hooks";
import { CreateIssueDialog } from "@/features/projects/ui/CreateIssueDialog";
import { ProjectIssuesPanel } from "@/features/projects/ui/ProjectIssuesPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import type { UserProfileLookup } from "@/features/profile/lib/identity";

type ChannelIssuesAuxiliaryPanelProps = {
  activeChannel: Channel;
  canResetWidth: boolean;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  profiles?: UserProfileLookup;
  widthPx: number;
};

/**
 * Channel-scoped issue surface. The channel binding on a repository is the
 * authority for scope: no binding means no issue list, never a global fallback.
 */
export function channelIssuesPanelInstanceKey(channelId: string) {
  return `channel-issues-panel:${channelId}`;
}

export function ChannelIssuesPanelHeader({
  onBack,
  onClose,
  repositoryName,
  selectedIssueId,
}: {
  onBack: () => void;
  onClose: () => void;
  repositoryName: string | null;
  selectedIssueId: string | null;
}) {
  return (
    <header className="relative z-40 flex shrink-0 items-center justify-between border-b border-border/60 px-4 py-3">
      <div className="flex min-w-0 items-center gap-2">
        {selectedIssueId ? (
          <Button
            aria-label="Back to issues"
            onClick={onBack}
            size="sm"
            title="Back to issues"
            type="button"
            variant="ghost"
          >
            <ArrowLeft className="h-4 w-4" />
            <span>Back to issues</span>
          </Button>
        ) : null}
        <div className="min-w-0">
          <h2 className="text-sm font-semibold">Issues</h2>
          <p className="truncate text-xs text-muted-foreground">
            {repositoryName ?? "No repository linked to this channel"}
          </p>
        </div>
      </div>
      <Button
        aria-label="Close issues"
        onClick={onClose}
        size="icon"
        title="Close issues"
        type="button"
        variant="ghost"
      >
        <X className="h-4 w-4" />
      </Button>
    </header>
  );
}

export function ChannelIssuesAuxiliaryPanel(
  props: ChannelIssuesAuxiliaryPanelProps,
) {
  return (
    <ChannelIssuesAuxiliaryPanelForChannel
      key={channelIssuesPanelInstanceKey(props.activeChannel.id)}
      {...props}
    />
  );
}

function ChannelIssuesAuxiliaryPanelForChannel({
  activeChannel,
  canResetWidth,
  onClose,
  onResetWidth,
  onResizeStart,
  profiles,
  widthPx,
}: ChannelIssuesAuxiliaryPanelProps) {
  const projectsQuery = useProjectsQuery();
  const repository = projectsQuery.data
    ?.flatMap((project) => project.repositories)
    .find((candidate) => candidate.channelId === activeChannel.id);
  const [selectedIssueId, setSelectedIssueId] = React.useState<string | null>(
    null,
  );
  const [createIssueOpen, setCreateIssueOpen] = React.useState(false);
  const bwActivation = useProjectBwActivation(repository?.repoAddress);
  const bwActive = bwActivation.data === true;
  const legacyIssueMutation = useCreateProjectIssueMutation(repository);
  const bwIssueMutation = useCreateProjectBwIssueMutation(repository);
  const createIssueMutation = bwActive ? bwIssueMutation : legacyIssueMutation;

  const handleCreateIssue = React.useCallback(
    async ({
      acceptanceCriteria,
      body,
      nonGoals,
      title,
    }: {
      acceptanceCriteria?: string[];
      body: string;
      nonGoals?: string[];
      title: string;
    }) => {
      try {
        await createIssueMutation.mutateAsync({
          acceptanceCriteria,
          body,
          nonGoals,
          title,
        });
      } catch (error) {
        if (!(error instanceof BwIssueCreatedButNotEnrolledError)) throw error;
      }
      await projectsQuery.refetch();
      setCreateIssueOpen(false);
    },
    [createIssueMutation, projectsQuery],
  );

  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId="channel-issues-auxiliary-pane"
      widthPx={widthPx}
    >
      <div className="flex min-h-0 flex-1 flex-col">
        <ChannelIssuesPanelHeader
          onBack={() => setSelectedIssueId(null)}
          onClose={onClose}
          repositoryName={repository?.name ?? null}
          selectedIssueId={selectedIssueId}
        />
        <div className="min-h-0 flex-1 overflow-y-auto">
          {projectsQuery.isLoading ? (
            <p className="p-4 text-sm text-muted-foreground">Loading issues…</p>
          ) : repository ? (
            <>
              {!selectedIssueId ? (
                <div className="flex justify-end px-4 pt-3">
                  <Button
                    onClick={() => setCreateIssueOpen(true)}
                    size="sm"
                    type="button"
                  >
                    <Plus className="h-4 w-4" />
                    Create issue
                  </Button>
                </div>
              ) : null}
              <ProjectIssuesPanel
                onSelectedIssueIdChange={setSelectedIssueId}
                profiles={profiles}
                project={repository}
                selectedIssueId={selectedIssueId}
              />
              <CreateIssueDialog
                isCreating={createIssueMutation.isPending}
                onCreate={handleCreateIssue}
                onOpenChange={setCreateIssueOpen}
                open={createIssueOpen}
                projectName={repository.name}
                templates={bwActive ? BW_ISSUE_TEMPLATES : undefined}
              />
            </>
          ) : (
            <p className="p-4 text-sm text-muted-foreground">
              Link a repository to this channel to keep its issue work here.
            </p>
          )}
        </div>
      </div>
    </RightAuxiliaryPane>
  );
}
