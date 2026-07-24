import { Check, ChevronDown, GitBranch } from "lucide-react";

import type { Project, Repository } from "@/features/projects/hooks";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function ProjectRepositoryPicker({
  onChange,
  project,
  repository,
}: {
  onChange: (repositoryId: string) => void;
  project: Project;
  repository: Repository;
}) {
  if (project.repositories.length < 2) return null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label="Select repository"
          className="h-8 max-w-64 shrink-0 gap-1.5"
          data-testid="project-repository-picker"
          size="sm"
          variant="outline"
        >
          <GitBranch className="h-3.5 w-3.5" />
          <span className="truncate">{repository.name}</span>
          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-56">
        {project.repositories.map((candidate) => (
          <DropdownMenuItem
            className="justify-between gap-4"
            data-testid={`project-repository-${candidate.dtag}`}
            key={candidate.id}
            onSelect={() => onChange(candidate.id)}
          >
            <span className="truncate">{candidate.name}</span>
            {candidate.id === repository.id ? (
              <Check className="h-4 w-4 shrink-0" />
            ) : null}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
