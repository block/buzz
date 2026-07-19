import * as React from "react";
import { toast } from "sonner";

import {
  type Project,
  useProjectPullRequestsQuery,
  useRepoStateQuery,
} from "@/features/projects/hooks";
import { useCreateProjectPullRequestMutation } from "@/features/projects/pullRequestMutations";
import { useProjectRepoSyncStatusQuery } from "@/features/projects/repoSyncHooks";

import {
  CreateProjectWorkItemDialog,
  type CreateProjectWorkItemDialogInput,
} from "./CreateProjectWorkItemDialog";

export type CreatePullRequestDialogInput = CreateProjectWorkItemDialogInput;

export function CreatePullRequestDialog({
  initialProjectId,
  onCreated,
  onOpenChange,
  open,
  projects,
  reposDir,
}: {
  initialProjectId?: string;
  onCreated: (project: Project, pullRequestId: string) => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  projects: Project[];
  reposDir?: string | null;
}) {
  const initialProject =
    projects.find((project) => project.id === initialProjectId) ?? projects[0];
  const [projectId, setProjectId] = React.useState(initialProject?.id ?? "");
  const project =
    projects.find((candidate) => candidate.id === projectId) ?? initialProject;
  const repoStateQuery = useRepoStateQuery(project);
  const pullRequestsQuery = useProjectPullRequestsQuery(project);
  const initialSyncQuery = useProjectRepoSyncStatusQuery(
    project,
    reposDir,
    project?.defaultBranch,
  );
  const branchOptions = React.useMemo(() => {
    const names = [
      project?.defaultBranch,
      ...(repoStateQuery.data?.branches.map((branch) => branch.name) ?? []),
      initialSyncQuery.data?.localBranch,
    ].filter((name): name is string => Boolean(name));
    return [...new Set(names)];
  }, [
    initialSyncQuery.data?.localBranch,
    project?.defaultBranch,
    repoStateQuery.data?.branches,
  ]);
  const [targetBranch, setTargetBranch] = React.useState(
    project?.defaultBranch ?? "",
  );
  const [sourceBranch, setSourceBranch] = React.useState("");
  const sourceSyncQuery = useProjectRepoSyncStatusQuery(
    project,
    reposDir,
    sourceBranch || null,
    targetBranch || null,
  );
  const createMutation = useCreateProjectPullRequestMutation(project);

  React.useEffect(() => {
    if (!open) return;
    const nextProject =
      projects.find((candidate) => candidate.id === initialProjectId) ??
      projects[0];
    setProjectId(nextProject?.id ?? "");
  }, [initialProjectId, open, projects]);

  React.useEffect(() => {
    if (!project) return;
    setTargetBranch(project.defaultBranch);
    setSourceBranch("");
  }, [project]);

  React.useEffect(() => {
    if (
      sourceBranch &&
      branchOptions.includes(sourceBranch) &&
      sourceBranch !== targetBranch
    ) {
      return;
    }
    setSourceBranch(
      branchOptions.find((branch) => branch !== targetBranch) ?? "",
    );
  }, [branchOptions, sourceBranch, targetBranch]);

  const sourceCommit =
    repoStateQuery.data?.branches.find((branch) => branch.name === sourceBranch)
      ?.commit ??
    (sourceSyncQuery.data?.remoteBranch === sourceBranch
      ? sourceSyncQuery.data.remoteHead
      : null);
  const hasOpenPullRequest = (pullRequestsQuery.data ?? []).some(
    (pullRequest) =>
      (pullRequest.status === "Open" || pullRequest.status === "Draft") &&
      pullRequest.branchName === sourceBranch &&
      (pullRequest.targetBranch ?? project?.defaultBranch) === targetBranch,
  );
  const selectionError = !project
    ? "Choose a repository."
    : !targetBranch
      ? "Choose a base branch."
      : !sourceBranch
        ? "Choose a compare branch."
        : sourceBranch === targetBranch
          ? "The base and compare branches must be different."
          : hasOpenPullRequest
            ? "An open pull request already compares these branches."
            : !sourceCommit
              ? "The compare branch must be pushed before opening a pull request."
              : null;
  const description =
    project && sourceBranch && targetBranch
      ? `${project.name}: ${sourceBranch} → ${targetBranch}${sourceCommit ? ` at ${sourceCommit.slice(0, 7)}` : ""}`
      : "Choose a repository and branches to compare.";

  async function handleCreate(input: CreatePullRequestDialogInput) {
    if (!project || !sourceCommit || selectionError) {
      throw new Error(
        selectionError ?? "Pull request branches are incomplete.",
      );
    }
    const pullRequestId = await createMutation.mutateAsync({
      ...input,
      branch: sourceBranch,
      targetBranch,
      commit: sourceCommit,
      mergeBase: sourceSyncQuery.data?.mergeBase ?? null,
      reviewers: [],
    });
    toast.success("Pull request created.");
    await onCreated(project, pullRequestId);
  }

  return (
    <CreateProjectWorkItemDialog
      bodyPlaceholder="Add context for reviewers"
      description={description}
      isCreating={createMutation.isPending}
      itemName="pull-request"
      onCreate={handleCreate}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && createMutation.isPending) return;
        onOpenChange(nextOpen);
      }}
      open={open}
      submitDisabled={Boolean(selectionError)}
      title="Open a pull request"
      titlePlaceholder="Describe the change"
    >
      <div className="grid gap-3 rounded-xl border border-border/60 bg-muted/25 p-3 sm:grid-cols-2">
        <label className="space-y-1.5 text-sm font-medium sm:col-span-2">
          <span>Repository</span>
          <select
            className="h-10 w-full rounded-lg border border-input bg-background px-3 text-sm font-normal outline-hidden focus:ring-1 focus:ring-ring"
            data-testid="create-pull-request-repository"
            disabled={createMutation.isPending}
            onChange={(event) => setProjectId(event.target.value)}
            value={project?.id ?? ""}
          >
            {projects.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name}
              </option>
            ))}
          </select>
        </label>
        <label className="space-y-1.5 text-sm font-medium">
          <span>Base</span>
          <select
            className="h-10 w-full rounded-lg border border-input bg-background px-3 text-sm font-normal outline-hidden focus:ring-1 focus:ring-ring"
            data-testid="create-pull-request-base-branch"
            disabled={createMutation.isPending}
            onChange={(event) => setTargetBranch(event.target.value)}
            value={targetBranch}
          >
            {branchOptions.map((branch) => (
              <option key={branch} value={branch}>
                {branch}
              </option>
            ))}
          </select>
        </label>
        <label className="space-y-1.5 text-sm font-medium">
          <span>Compare</span>
          <select
            className="h-10 w-full rounded-lg border border-input bg-background px-3 text-sm font-normal outline-hidden focus:ring-1 focus:ring-ring"
            data-testid="create-pull-request-compare-branch"
            disabled={createMutation.isPending}
            onChange={(event) => setSourceBranch(event.target.value)}
            value={sourceBranch}
          >
            <option disabled value="">
              Select branch
            </option>
            {branchOptions.map((branch) => (
              <option key={branch} value={branch}>
                {branch}
              </option>
            ))}
          </select>
        </label>
        {selectionError ? (
          <p className="text-xs text-muted-foreground sm:col-span-2">
            {selectionError}
          </p>
        ) : null}
      </div>
    </CreateProjectWorkItemDialog>
  );
}
