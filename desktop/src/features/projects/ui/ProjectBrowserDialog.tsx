import { Check, Folders, Plus, Search } from "lucide-react";
import * as React from "react";

import type { Project } from "@/features/projects/hooks";
import type { CreateProjectInput } from "@/features/projects/useCreateProject";
import { useTranslation } from "@/i18n";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import { CreateProjectFormContent } from "./CreateProjectFormContent";

function projectSearchText(project: Project) {
  return [
    project.name,
    project.description,
    ...project.repositories.flatMap((repository) => [
      repository.name,
      repository.description,
    ]),
  ]
    .filter(Boolean)
    .join(" ")
    .toLocaleLowerCase();
}

export function ProjectBrowserDialog({
  isCreating,
  onCreate,
  onOpenChange,
  onSelectProject,
  open,
  projects,
  selectedProjectAddresses,
}: {
  isCreating: boolean;
  onCreate: (input: CreateProjectInput) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  onSelectProject: (project: Project) => void;
  open: boolean;
  projects: readonly Project[];
  selectedProjectAddresses: ReadonlySet<string>;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = React.useState<"browse" | "create">("browse");
  const [query, setQuery] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);
  const deferredQuery = React.useDeferredValue(
    query.trim().toLocaleLowerCase(),
  );

  React.useEffect(() => {
    if (!open) return;
    setMode("browse");
    setQuery("");
    const timerId = globalThis.setTimeout(() => inputRef.current?.focus(), 50);
    return () => globalThis.clearTimeout(timerId);
  }, [open]);

  const visibleProjects = React.useMemo(() => {
    const sorted = [...projects].sort((left, right) =>
      left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
    );
    if (!deferredQuery) return sorted;
    return sorted.filter((project) =>
      projectSearchText(project).includes(deferredQuery),
    );
  }, [deferredQuery, projects]);
  const hasExactMatch = projects.some(
    (project) =>
      project.name.trim().toLocaleLowerCase() ===
      query.trim().toLocaleLowerCase(),
  );

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isCreating) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      {mode === "create" ? (
        <CreateProjectFormContent
          active={open}
          initialName={query.trim()}
          isCreating={isCreating}
          onBack={() => setMode("browse")}
          onCreate={onCreate}
          onCreated={() => onOpenChange(false)}
        />
      ) : (
        <ChooserDialogContent
          className="max-w-lg"
          contentClassName="space-y-3 pt-1"
          data-testid="project-browser-dialog"
          headerClassName="pb-2"
          scrollAreaClassName="px-3"
          title={t("projects.browser-dialog.title")}
        >
          <div className={MODAL_SEARCH_SHELL_CLASS}>
            <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
            <input
              aria-label={t("projects.browser-dialog.search-aria")}
              className={MODAL_SEARCH_INPUT_CLASS}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("projects.browser-dialog.search-placeholder")}
              ref={inputRef}
              type="search"
              value={query}
            />
          </div>

          {!hasExactMatch ? (
            <Button
              className="h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-left"
              data-testid="project-browser-create"
              onClick={() => setMode("create")}
              type="button"
              variant="ghost"
            >
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                <Plus className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-medium text-foreground">
                  {query.trim()
                    ? t("projects.browser-dialog.create-query", {
                        query: query.trim(),
                      })
                    : t("projects.create-project-form.title")}
                </span>
                <span className="block text-xs font-normal text-muted-foreground">
                  {t("projects.browser-dialog.create-hint")}
                </span>
              </span>
            </Button>
          ) : null}

          <div className="border-t border-border/60 pt-2">
            {visibleProjects.length > 0 ? (
              <div className="space-y-0.5">
                {visibleProjects.map((project) => (
                  <Button
                    className="h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-left"
                    data-testid={`project-browser-result-${project.dtag}`}
                    key={project.id}
                    onClick={() => {
                      onOpenChange(false);
                      onSelectProject(project);
                    }}
                    type="button"
                    variant="ghost"
                  >
                    <Folders className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium text-foreground">
                        {project.name}
                      </span>
                      <span className="block truncate text-xs font-normal text-muted-foreground">
                        {project.description ||
                          `${project.repositories.length} ${
                            project.repositories.length === 1
                              ? "repository"
                              : "repositories"
                          }`}
                      </span>
                    </span>
                    {selectedProjectAddresses.has(project.projectAddress) ? (
                      <span className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                        <Check className="h-3.5 w-3.5" />
                        {t("sidebar.projects.filter-added")}
                      </span>
                    ) : null}
                  </Button>
                ))}
              </div>
            ) : (
              <div className="px-4 py-10 text-center">
                <p className="text-sm font-medium text-foreground">
                  {t("projects.browser-dialog.empty-title")}
                </p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("projects.browser-dialog.empty-body")}
                </p>
              </div>
            )}
          </div>
        </ChooserDialogContent>
      )}
    </Dialog>
  );
}
