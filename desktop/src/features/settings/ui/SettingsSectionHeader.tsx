import type { ReactNode } from "react";

import { PageHeader } from "@/shared/ui/PageHeader";
import { cn } from "@/shared/lib/cn";

/**
 * Page title for a Settings card. Thin wrapper over the shared {@link PageHeader}
 * that preserves the `mb-12` spacing every Settings card relies on.
 */
export function SettingsSectionHeader({
  action,
  className,
  description,
  title,
}: {
  action?: ReactNode;
  className?: string;
  description?: ReactNode;
  title: ReactNode;
}) {
  return (
    <PageHeader
      action={action}
      className={cn("mb-12", className)}
      description={
        description ? (
          <span data-settings-subcopy className="text-muted-foreground/70">
            {description}
          </span>
        ) : undefined
      }
      title={title}
    />
  );
}
