import * as React from "react";
import { toast } from "sonner";

import type { Project, Repository } from "@/features/projects/hooks";
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
  const createMutation = useCreateProjectIssueMutation(repository);

  React.useEffect(() => {
    if (!open) return;
    const nextProject =
      projects.find((candidate) => candidate.id === initialProjectId) ??
      projects[0];
    setRepositoryId(selectProjectRepository(nextProject, null)?.id ?? "");
  }, [initialProjectId, open, projects]);

  async function handleCreate(input: CreateProjectWorkItemDialogInput) {
    if (!project || !repository) throw new Error("Choose a repository.");
    const issueId = await createMutation.mutateAsync(input);
    toast.success("Issue created.");
    await onCreated(project, repository, issueId);
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
