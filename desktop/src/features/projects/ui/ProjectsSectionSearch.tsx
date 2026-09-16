import { Search, X } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

import type {
  ProjectsFilter,
  ProjectsSort,
} from "@/features/projects/lib/projectsViewHelpers";
import { ProjectsSortSelect } from "@/features/projects/ui/ProjectsListHeaderBar";
import { projectsSectionTitle } from "@/features/projects/ui/projectsSectionMeta";
import { ProjectsToolbar } from "@/features/projects/ui/ProjectsToolbar";
import { Button } from "@/shared/ui/button";

export function ProjectsSectionSearch({
  filter,
  onFilterChange,
  onQueryChange,
  onSortChange,
  sort,
}: {
  filter: ProjectsFilter;
  onFilterChange: (filter: ProjectsFilter) => void;
  onQueryChange: (query: string) => void;
  onSortChange: (sort: ProjectsSort) => void;
  sort: ProjectsSort;
}) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const deferredQuery = React.useDeferredValue(query);
  const focusFrameRef = React.useRef<number | null>(null);
  const reduceMotion = useReducedMotion();
  const transition = {
    duration: reduceMotion ? 0 : 0.06,
    ease: [0.2, 0.8, 0.2, 1] as const,
  };
  const close = React.useCallback(() => {
    setOpen(false);
    setQuery("");
    onQueryChange("");
  }, [onQueryChange]);

  React.useEffect(() => {
    onQueryChange(deferredQuery);
  }, [deferredQuery, onQueryChange]);
  const focusSearchInput = React.useCallback(
    (input: HTMLInputElement | null) => {
      if (!input) return;
      focusFrameRef.current = window.requestAnimationFrame(() => input.focus());
    },
    [],
  );
  React.useEffect(
    () => () => {
      if (focusFrameRef.current !== null) {
        window.cancelAnimationFrame(focusFrameRef.current);
      }
    },
    [],
  );

  return (
    <div className="flex min-w-0 flex-1 items-center gap-1.5">
      <Button
        aria-label={
          open
            ? "Close project search"
            : `Search ${projectsSectionTitle(filter)}`
        }
        className="group/projects-search relative h-7 w-7 shrink-0 rounded-full border border-border/55 bg-transparent px-0 text-muted-foreground shadow-none hover:border-border hover:bg-muted/25 hover:text-foreground focus-visible:border-border"
        data-testid={
          open ? "projects-section-search-close" : "projects-activity-search"
        }
        onClick={open ? close : () => setOpen(true)}
        size="icon"
        title={open ? "Close search" : `Search ${projectsSectionTitle(filter)}`}
        type="button"
        variant="ghost"
      >
        <span className="relative flex h-4 w-4 items-center justify-center">
          <Search
            className={
              open
                ? "h-4 w-4 transition-[opacity,scale] duration-75 group-hover/projects-search:scale-75 group-hover/projects-search:opacity-0"
                : "h-4 w-4"
            }
          />
          {open ? (
            <X className="absolute h-3 w-3 scale-75 opacity-0 transition-[opacity,scale] duration-75 group-hover/projects-search:scale-100 group-hover/projects-search:opacity-100" />
          ) : null}
        </span>
      </Button>
      <div className="relative flex h-9 min-w-0 flex-1 items-center overflow-hidden">
        <AnimatePresence initial={false} mode="wait">
          {open ? (
            <motion.div
              animate={{ clipPath: "inset(0 0% 0 0)", opacity: 1 }}
              className="absolute inset-0 flex min-w-0 items-center overflow-hidden"
              data-testid="projects-section-search"
              exit={{ clipPath: "inset(0 100% 0 0)", opacity: 0 }}
              initial={{ clipPath: "inset(0 100% 0 0)", opacity: 0 }}
              key="search"
              transition={transition}
            >
              <input
                aria-label={`Search ${projectsSectionTitle(filter)}`}
                className="min-w-0 flex-1 bg-transparent px-2 pr-36 text-sm text-foreground outline-none placeholder:text-muted-foreground/55"
                data-testid="projects-section-search-input"
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Escape") return;
                  event.preventDefault();
                  close();
                }}
                placeholder={`Search ${projectsSectionTitle(filter).toLocaleLowerCase()}`}
                ref={focusSearchInput}
                type="search"
                value={query}
              />
              {filter !== "all" && filter !== "channels" ? (
                <div className="absolute inset-y-0 right-0 flex w-32 items-center justify-end">
                  <ProjectsSortSelect onChange={onSortChange} sort={sort} />
                </div>
              ) : null}
            </motion.div>
          ) : (
            <motion.div
              animate="visible"
              className="absolute inset-0 flex min-w-0 items-center"
              exit="hidden"
              initial="hidden"
              key="navigation"
              variants={{
                hidden: {
                  transition: {
                    staggerChildren: reduceMotion ? 0 : 0.006,
                    staggerDirection: -1,
                  },
                },
                visible: {
                  transition: {
                    staggerChildren: reduceMotion ? 0 : 0.006,
                  },
                },
              }}
            >
              <ProjectsToolbar
                filter={filter}
                onFilterChange={onFilterChange}
                reduceMotion={Boolean(reduceMotion)}
              />
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}
