import { type LucideIcon, Plus } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

export function ProjectSectionHeader({
  action,
  className,
  icon: Icon,
  testId = "project-section-header",
  title,
}: {
  action?: {
    disabled?: boolean;
    label: string;
    onClick: () => void;
    title?: string;
  };
  className?: string;
  icon: LucideIcon;
  testId?: string;
  title: string;
}) {
  return (
    <header
      className={cn("flex min-h-12 items-center gap-3 px-4 py-2", className)}
      data-testid={testId}
    >
      <Icon
        className="h-4 w-4 shrink-0 text-muted-foreground"
        data-testid="project-section-header-icon"
      />
      <h2 className="min-w-0 flex-1 truncate text-sm font-semibold text-foreground">
        {title}
      </h2>
      {action ? (
        <Button
          aria-label={action.label}
          className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
          disabled={action.disabled}
          onClick={action.onClick}
          size="icon"
          title={action.title ?? action.label}
          variant="ghost"
        >
          <Plus className="h-4 w-4" />
        </Button>
      ) : null}
    </header>
  );
}
