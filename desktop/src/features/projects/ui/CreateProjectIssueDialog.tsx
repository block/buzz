import * as React from "react";
import { toast } from "sonner";

import type { Project } from "@/features/projects/hooks";
import { useCreateProjectIssueMutation } from "@/features/projects/issueMutations";
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
  onCreated: (project: Project, issueId: string) => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  projects: Project[];
}) {
  const initialProject =
    projects.find((project) => project.id === initialProjectId) ?? projects[0];
  const [projectId, setProjectId] = React.useState(initialProject?.id ?? "");
  const project =
    projects.find((candidate) => candidate.id === projectId) ?? initialProject;
  const createMutation = useCreateProjectIssueMutation(project);

  React.useEffect(() => {
    if (!open) return;
    const nextProject =
      projects.find((candidate) => candidate.id === initialProjectId) ??
      projects[0];
    setProjectId(nextProject?.id ?? "");
  }, [initialProjectId, open, projects]);

  async function handleCreate(input: CreateProjectWorkItemDialogInput) {
    if (!project) throw new Error("Choose a repository.");
    const issueId = await createMutation.mutateAsync(input);
    toast.success("Issue created.");
    await onCreated(project, issueId);
  }

  return (
    <CreateProjectWorkItemDialog
      bodyPlaceholder="Add context, expected behavior, or reproduction steps"
      description={
        project
          ? `Create an issue in ${project.name}`
          : "Choose a repository for this issue."
      }
      isCreating={createMutation.isPending}
      itemName="issue"
      onCreate={handleCreate}
      onOpenChange={onOpenChange}
      open={open}
      submitDisabled={!project}
      title="Create an issue"
      titlePlaceholder="Describe the issue"
    >
      <label className="block space-y-1.5 text-sm font-medium">
        <span>Repository</span>
        <select
          className="h-10 w-full rounded-lg border border-input bg-background px-3 text-sm font-normal outline-hidden focus:ring-1 focus:ring-ring"
          data-testid="create-issue-repository"
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
    </CreateProjectWorkItemDialog>
  );
}
