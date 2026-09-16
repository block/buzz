import { X } from "lucide-react";

import type { ProjectDetailAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import { Button } from "@/shared/ui/button";
import { PROJECT_COLUMN_HEADER_BACKDROP_CLASS } from "./projectPanelStyles";

function contextLabel(context: ProjectDetailAgentContext) {
  if (context.selection?.length) {
    return "Agent chat";
  }
  if (context.workItem) {
    return context.workItem.title;
  }
  if (context.file?.path) {
    return context.file.path.split("/").filter(Boolean).at(-1) ?? context.view;
  }
  return context.view;
}

export function ProjectAgentContextStrip({
  context,
  onClose,
  sharedBackdrop = false,
}: {
  context: ProjectDetailAgentContext;
  onClose?: () => void;
  sharedBackdrop?: boolean;
}) {
  const label = contextLabel(context);

  return (
    <div
      className={`absolute inset-x-0 top-0 z-30 flex h-13 shrink-0 items-center px-4 ${
        sharedBackdrop ? "" : PROJECT_COLUMN_HEADER_BACKDROP_CLASS
      }`}
      data-testid="project-agent-context"
      title={label}
    >
      <p className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
        {label}
      </p>
      {onClose ? (
        <Button
          aria-label="Close agent chat"
          className="h-7 w-7 shrink-0"
          onClick={onClose}
          size="icon"
          title="Close agent chat"
          type="button"
          variant="ghost"
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      ) : null}
    </div>
  );
}
