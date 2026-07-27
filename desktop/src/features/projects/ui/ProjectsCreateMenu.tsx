import { CircleDot, FolderGit2, GitPullRequest, Plus } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import {
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";

const MENU_ITEM_CLASS =
  "flex min-h-9 w-full items-center gap-2 rounded-lg py-2 pl-2 pr-4 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus:bg-muted/50 focus:text-foreground focus-visible:ring-1 focus-visible:ring-ring [&_svg]:size-4 [&_svg]:shrink-0";

export function ProjectsCreateMenu({
  onCreateIssue,
  onCreatePullRequest,
  onCreateRepository,
}: {
  onCreateIssue: () => void;
  onCreatePullRequest: () => void;
  onCreateRepository: () => void;
}) {
  const [open, setOpen] = React.useState(false);
  const containerRef = React.useRef<HTMLElement>(null);

  React.useEffect(() => {
    if (!open) return;
    function handlePointerDown(event: PointerEvent) {
      if (!containerRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    globalThis.document.addEventListener(
      "pointerdown",
      handlePointerDown,
      true,
    );
    return () =>
      globalThis.document.removeEventListener(
        "pointerdown",
        handlePointerDown,
        true,
      );
  }, [open]);

  function select(action: () => void) {
    setOpen(false);
    action();
  }

  return (
    <nav
      aria-label="Create project item"
      className="relative shrink-0 pl-4"
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          setOpen(false);
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          setOpen(false);
          containerRef.current?.querySelector<HTMLElement>("button")?.focus();
        }
      }}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
      ref={containerRef}
    >
      <Button
        aria-expanded={open}
        aria-haspopup="menu"
        aria-label="Create"
        className="h-8 w-8 rounded-full"
        data-testid="projects-create-menu"
        onClick={() => setOpen(true)}
        size="icon"
        type="button"
        variant="default"
      >
        <Plus className="h-4 w-4" />
      </Button>
      {open ? (
        <div className="absolute right-0 top-full z-50 min-w-48 pt-1">
          <div
            className={`rounded-xl p-1 ${POPOVER_SURFACE_CLASS}`}
            role="menu"
            style={POPOVER_SHADOW_STYLE}
          >
            <button
              className={MENU_ITEM_CLASS}
              onClick={() => select(onCreateRepository)}
              role="menuitem"
              type="button"
            >
              <FolderGit2 />
              Repository
            </button>
            <button
              className={MENU_ITEM_CLASS}
              onClick={() => select(onCreateIssue)}
              role="menuitem"
              type="button"
            >
              <CircleDot />
              Issue
            </button>
            <button
              className={MENU_ITEM_CLASS}
              onClick={() => select(onCreatePullRequest)}
              role="menuitem"
              type="button"
            >
              <GitPullRequest />
              Pull Request
            </button>
          </div>
        </div>
      ) : null}
    </nav>
  );
}
