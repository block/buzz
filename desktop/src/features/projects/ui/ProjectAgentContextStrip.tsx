import type { ProjectDetailAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import { PROJECT_COLUMN_HEADER_BACKDROP_CLASS } from "./projectPanelStyles";

function contextLabel(context: ProjectDetailAgentContext) {
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
  sharedBackdrop = false,
}: {
  context: ProjectDetailAgentContext;
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
      <p className="truncate text-sm font-medium text-foreground">{label}</p>
    </div>
  );
}
