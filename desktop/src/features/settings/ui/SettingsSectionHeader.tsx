import type { ReactNode } from "react";

import { PageHeader } from "@/shared/ui/PageHeader";

/**
 * Page title for a Settings card. Thin wrapper over the shared {@link PageHeader}
 * that preserves the `mb-12` spacing every Settings card relies on.
 */
export function SettingsSectionHeader({
  action,
  description,
  title,
}: {
  action?: ReactNode;
  description: ReactNode;
  title: ReactNode;
}) {
  return (
    <PageHeader
      action={action}
      className="mb-12"
      description={description}
      title={title}
    />
  );
}
