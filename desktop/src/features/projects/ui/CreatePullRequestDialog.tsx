import * as React from "react";
import { toast } from "sonner";

import {
  type Project,
  type Repository,
  useProjectPullRequestsQuery,
  useRepoStateQuery,
} from "@/features/projects/hooks";
import { selectProjectRepository } from "@/features/projects/projectModels";
import { useCreateProjectPullRequestMutation } from "@/features/projects/pullRequestMutations";
import { useProjectRepoSyncStatusQuery } from "@/features/projects/repoSyncHooks";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/ui/select";
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
  onCreated: (
    project: Project,
    repository: Repository,
    pullRequestId: string,
  ) => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  projects: Project[];
  reposDir?: string | null;
}) {
  const repositoryOptions = React.useMemo(
    () =>
      projects.flatMap((project) =>
        project.repositories.map((repository) => ({ project, repository })),
      ),
    [projects],
  );
  const initialProject =
    projects.find((project) => project.id === initialProjectId) ?? projects[0];
  const initialRepository = selectProjectRepository(initialProject, null);
  const [repositoryId, setRepositoryId] = React.useState(
    initialRepository?.id ?? "",
  );
  const selection =
    repositoryOptions.find(
      (candidate) => candidate.repository.id === repositoryId,
    ) ?? repositoryOptions[0];
  const project = selection?.project;
  const repository = selection?.repository;
  const repoStateQuery = useRepoStateQuery(repository);
  const pullRequestsQuery = useProjectPullRequestsQuery(repository);
  const initialSyncQuery = useProjectRepoSyncStatusQuery(
    repository,
    reposDir,
    repository?.defaultBranch,
  );
  const branchOptions = React.useMemo(() => {
    const names = [
      repository?.defaultBranch,
      ...(repoStateQuery.data?.branches.map((branch) => branch.name) ?? []),
      initialSyncQuery.data?.localBranch,
    ].filter((name): name is string => Boolean(name));
    return [...new Set(names)];
  }, [
    initialSyncQuery.data?.localBranch,
    repository?.defaultBranch,
    repoStateQuery.data?.branches,
  ]);
  const [targetBranch, setTargetBranch] = React.useState(
    repository?.defaultBranch ?? "",
  );
  const [sourceBranch, setSourceBranch] = React.useState("");
  const sourceSyncQuery = useProjectRepoSyncStatusQuery(
    repository,
    reposDir,
    sourceBranch || null,
    targetBranch || null,
  );
  const createMutation = useCreateProjectPullRequestMutation(repository);

  React.useEffect(() => {
    if (!open) return;
    const nextProject =
      projects.find((candidate) => candidate.id === initialProjectId) ??
      projects[0];
    setRepositoryId(selectProjectRepository(nextProject, null)?.id ?? "");
  }, [initialProjectId, open, projects]);

  React.useEffect(() => {
    if (!repository) return;
    setTargetBranch(repository.defaultBranch);
    setSourceBranch("");
  }, [repository]);

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
      (pullRequest.targetBranch ?? repository?.defaultBranch) === targetBranch,
  );
  const selectionError = !repository
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
    repository && sourceBranch && targetBranch
      ? `${repository.name}: ${sourceBranch} → ${targetBranch}${sourceCommit ? ` at ${sourceCommit.slice(0, 7)}` : ""}`
      : "Choose a repository and branches to compare.";

  async function handleCreate(input: CreatePullRequestDialogInput) {
    if (!project || !repository || !sourceCommit || selectionError) {
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
    await onCreated(project, repository, pullRequestId);
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
        <div className="space-y-1.5 text-sm font-medium sm:col-span-2">
          <label htmlFor="create-pull-request-repository">Repository</label>
          <Select
            disabled={createMutation.isPending}
            onValueChange={setRepositoryId}
            value={repository?.id ?? ""}
          >
            <SelectTrigger
              className="h-10 rounded-lg px-3 py-0"
              data-testid="create-pull-request-repository"
              id="create-pull-request-repository"
            >
              <SelectValue placeholder="Select a repository" />
            </SelectTrigger>
            <SelectContent>
              {repositoryOptions.map((candidate) => (
                <SelectItem
                  key={candidate.repository.id}
                  value={candidate.repository.id}
                >
                  {candidate.project.repositories.length > 1
                    ? `${candidate.project.name} / ${candidate.repository.name}`
                    : candidate.project.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5 text-sm font-medium">
          <label htmlFor="create-pull-request-base-branch">Base</label>
          <Select
            disabled={createMutation.isPending}
            onValueChange={setTargetBranch}
            value={targetBranch}
          >
            <SelectTrigger
              className="h-10 rounded-lg px-3 py-0"
              data-testid="create-pull-request-base-branch"
              id="create-pull-request-base-branch"
            >
              <SelectValue placeholder="Select a branch" />
            </SelectTrigger>
            <SelectContent>
              {branchOptions.map((branch) => (
                <SelectItem key={branch} value={branch}>
                  {branch}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5 text-sm font-medium">
          <label htmlFor="create-pull-request-compare-branch">Compare</label>
          <Select
            disabled={createMutation.isPending}
            onValueChange={setSourceBranch}
            value={sourceBranch}
          >
            <SelectTrigger
              className="h-10 rounded-lg px-3 py-0"
              data-testid="create-pull-request-compare-branch"
              id="create-pull-request-compare-branch"
            >
              <SelectValue placeholder="Select branch" />
            </SelectTrigger>
            <SelectContent>
              {branchOptions.map((branch) => (
                <SelectItem key={branch} value={branch}>
                  {branch}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {selectionError ? (
          <p className="text-xs text-muted-foreground sm:col-span-2">
            {selectionError}
          </p>
        ) : null}
      </div>
    </CreateProjectWorkItemDialog>
  );
}
