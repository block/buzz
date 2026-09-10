import * as React from "react";
import { toast } from "sonner";

import type { Project, Repository } from "@/features/projects/hooks";
import {
  BwIssueCreatedButNotEnrolledError,
  useCreateProjectBwIssueMutation,
  useProjectBwActivation,
} from "@/features/projects/bwIssueCreation";
import { BW_ISSUE_TEMPLATES } from "@/features/projects/bwIssueTemplates";
import { useCreateProjectIssueMutation } from "@/features/projects/issueMutations";
import { selectProjectRepository } from "@/features/projects/projectModels";
import {
  CreateProjectWorkItemDialog,
  type CreateProjectWorkItemDialogInput,
} from "./CreateProjectWorkItemDialog";

export function CreateProjectIssueDialog({
  initialProjectId,
  onCreated,
  onOpenChange,
  open,
  projects,
}: {
  initialProjectId?: string;
  onCreated: (
    project: Project,
    repository: Repository,
    issueId: string,
  ) => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  projects: Project[];
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
  const [repositoryId, setRepositoryId] = React.useState(
    selectProjectRepository(initialProject, null)?.id ?? "",
  );
  const selection =
    repositoryOptions.find(
      (candidate) => candidate.repository.id === repositoryId,
    ) ?? repositoryOptions[0];
  const project = selection?.project;
  const repository = selection?.repository;
  const bwActivation = useProjectBwActivation(repository?.repoAddress);
  const bwActive = bwActivation.data === true;
  const legacyMutation = useCreateProjectIssueMutation(repository);
  const bwMutation = useCreateProjectBwIssueMutation(repository);
  const createMutation = bwActive ? bwMutation : legacyMutation;

  React.useEffect(() => {
    if (!open) return;
    const nextProject =
      projects.find((candidate) => candidate.id === initialProjectId) ??
      projects[0];
    setRepositoryId(selectProjectRepository(nextProject, null)?.id ?? "");
  }, [initialProjectId, open, projects]);

  async function handleCreate(input: CreateProjectWorkItemDialogInput) {
    if (!project || !repository) throw new Error("Choose a repository.");
    try {
      const issueId = await createMutation.mutateAsync(input);
      toast.success("Issue created.");
      await onCreated(project, repository, issueId);
    } catch (error) {
      // The root itself was already published and is real history; only the
      // enroll/patch attempt failed. Do not let a retry create a second
      // root — surface the reason and still navigate to the created issue.
      if (error instanceof BwIssueCreatedButNotEnrolledError) {
        toast.warning(
          `Issue created, but not yet a BW issue: ${error.message}`,
        );
        await onCreated(project, repository, error.issueId);
        return;
      }
      throw error;
    }
  }

  return (
    <CreateProjectWorkItemDialog
      bodyPlaceholder="Add context, expected behavior, or reproduction steps"
      description={
        repository
          ? `Create an issue in ${repository.name}`
          : "Choose a repository for this issue."
      }
      isCreating={createMutation.isPending}
      itemName="issue"
      onCreate={handleCreate}
      onOpenChange={onOpenChange}
      open={open}
      submitDisabled={!repository}
      templates={bwActive ? BW_ISSUE_TEMPLATES : undefined}
      title="Create an issue"
      titlePlaceholder="Describe the issue"
    >
      <label className="block space-y-1.5 text-sm font-medium">
        <span>Repository</span>
        <select
          className="h-10 w-full rounded-lg border border-input bg-background px-3 text-sm font-normal outline-hidden focus:ring-1 focus:ring-ring"
          data-testid="create-issue-repository"
          disabled={createMutation.isPending}
          onChange={(event) => setRepositoryId(event.target.value)}
          value={repository?.id ?? ""}
        >
          {repositoryOptions.map((candidate) => (
            <option
              key={candidate.repository.id}
              value={candidate.repository.id}
            >
              {candidate.project.repositories.length > 1
                ? `${candidate.project.name} / ${candidate.repository.name}`
                : candidate.project.name}
            </option>
          ))}
        </select>
      </label>
    </CreateProjectWorkItemDialog>
  );
}
