import {
  Check,
  ChevronDown,
  FolderGit2,
  FolderPlus,
  Link,
  Plus,
} from "lucide-react";

import type { Project, Repository } from "@/features/projects/hooks";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { PROJECT_PICKER_TRIGGER_CLASS } from "./projectPanelStyles";

export function ProjectRepositoryPicker({
  onAttach,
  onChange,
  onCreate,
  project,
  repository,
}: {
  onAttach?: () => void;
  onChange: (repositoryId: string) => void;
  onCreate?: () => void;
  project: Project;
  repository: Repository;
}) {
  const repositoryLabel = (
    <>
      <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span className="truncate">{repository.name}</span>
    </>
  );
  const unavailableRepositories = project.unavailableRepositoryAddresses ?? [];

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label="Select repository"
          className={cn("max-w-64", PROJECT_PICKER_TRIGGER_CLASS)}
          data-testid="project-repository-picker"
          size="sm"
          variant="outline"
        >
          {repositoryLabel}
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-56">
        <DropdownMenuLabel>Repositories</DropdownMenuLabel>
        {project.repositories.map((candidate) => (
          <DropdownMenuItem
            className="justify-between gap-4"
            data-testid={`project-repository-${candidate.dtag}`}
            key={candidate.id}
            onSelect={() => onChange(candidate.id)}
          >
            <span className="min-w-0 truncate">{candidate.name}</span>
            {candidate.id === repository.id ? (
              <Check className="h-4 w-4 shrink-0" />
            ) : null}
          </DropdownMenuItem>
        ))}
        {unavailableRepositories.map((address) => (
          <DropdownMenuItem
            className="justify-between gap-4"
            disabled
            key={address}
          >
            <span className="min-w-0 truncate">
              {address.slice(address.indexOf(":", 6) + 1)}
            </span>
            <span className="ml-auto text-xs text-muted-foreground">
              Unavailable
            </span>
          </DropdownMenuItem>
        ))}
        {onCreate && onAttach ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <DropdownMenuSubTrigger data-testid="add-project-repository">
                <Plus className="h-4 w-4" />
                Add repository
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                <DropdownMenuItem
                  data-testid="create-project-repository"
                  onSelect={onCreate}
                >
                  <FolderPlus className="h-4 w-4" />
                  Create new repository
                </DropdownMenuItem>
                <DropdownMenuItem
                  data-testid="attach-project-repository"
                  onSelect={onAttach}
                >
                  <Link className="h-4 w-4" />
                  Select existing repository
                </DropdownMenuItem>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
