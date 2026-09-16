import * as React from "react";

import { Button } from "@/shared/ui/button";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";
import { Sheet, SheetContent, SheetTitle } from "@/shared/ui/sheet";

export const ProjectsOverviewNarrowContextToggle = React.forwardRef<
  HTMLButtonElement,
  { onToggle: () => void; open: boolean }
>(({ onToggle, open }, ref) => (
  <Button
    aria-label={open ? "Hide project context" : "Show project context"}
    aria-pressed={open}
    className="h-7 w-7 text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
    data-testid="projects-overview-context-toggle"
    onClick={onToggle}
    ref={ref}
    size="icon"
    title={open ? "Hide project context" : "Show project context"}
    type="button"
    variant="ghost"
  >
    <DrawerPanelIcon
      className="-scale-x-100"
      side={open ? "left" : "right"}
      testId="projects-overview-context-icon"
    />
  </Button>
));
ProjectsOverviewNarrowContextToggle.displayName =
  "ProjectsOverviewNarrowContextToggle";

/**
 * Narrow-layout fallback for the Projects context rail: below the detached
 * breakpoint the rail cannot dock beside the content, so the same
 * section-following context panel opens as a dismissible right-hand sheet.
 * Radix supplies the focus trap, Escape handling, and close button.
 */
export function ProjectsOverviewContextSheet({
  children,
  onCloseAutoFocus,
  onOpenChange,
  open,
}: {
  children: React.ReactNode;
  onCloseAutoFocus?: (event: Event) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent
        aria-describedby={undefined}
        className="w-80 overflow-y-auto p-0 pt-10 sm:max-w-none"
        data-testid="projects-overview-context-sheet"
        onCloseAutoFocus={onCloseAutoFocus}
        side="right"
      >
        <SheetTitle className="sr-only">Project context</SheetTitle>
        {children}
      </SheetContent>
    </Sheet>
  );
}
