import type { Project, Repository } from "@/features/projects/hooks";
import { ProjectRepositoryManagement } from "./ProjectRepositoryManagement";

/**
 * Repository-scoped controls for the project detail chrome. Extracted from
 * `ProjectDetailScreen` to keep the screen under the file-size ratchet.
 */
export function ProjectDetailChromeActions({
  identityPubkey,
  onRepositoryChange,
  project,
  projects,
  repository,
}: {
  identityPubkey?: string;
  onRepositoryChange: (repositoryId: string) => void;
  project: Project;
  projects: Project[];
  repository: Repository;
}) {
  return (
    <ProjectRepositoryManagement
      identityPubkey={identityPubkey}
      onChange={onRepositoryChange}
      project={project}
      projects={projects}
      repository={repository}
    />
  );
}
