import type { Project, Repository } from "@/features/projects/hooks";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";

export function ProjectRepositoryHeaderControl({
  active,
  identityPubkey,
  onChange,
  project,
  projects,
  repository,
}: {
  active: boolean;
  identityPubkey?: string;
  onChange: (repositoryId: string) => void;
  project: Project;
  projects: Project[];
  repository: Repository;
}) {
  if (!active) return null;
  return (
    <div className="flex shrink-0 items-center gap-2">
      <span className="hidden text-xs font-medium text-muted-foreground sm:inline">
        Repository
      </span>
      <ProjectRepositoryManagement
        identityPubkey={identityPubkey}
        onChange={onChange}
        project={project}
        projects={projects}
        repository={repository}
      />
    </div>
  );
}
