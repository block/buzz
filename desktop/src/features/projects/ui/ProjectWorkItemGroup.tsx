import type * as React from "react";

import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
import { ProjectSelectableGroup } from "./ProjectSelectableGroup";

/** Groups related project work-item rows under a subtle status summary. */
export function ProjectWorkItemGroup({
  children,
  count,
  icon,
  items,
  label,
}: {
  children: React.ReactNode;
  count: number;
  icon: React.ReactNode;
  items: ProjectSelectionItem[];
  label: string;
}) {
  return (
    <ProjectSelectableGroup
      contentClassName="px-4"
      count={count}
      groupKey={label}
      headerClassName="mx-4 gap-3 px-4"
      headerTestId="project-work-item-group-header"
      icon={icon}
      items={items}
      label={label}
      labelTestId="project-work-item-group-label"
      testId="project-work-item-group"
    >
      {children}
    </ProjectSelectableGroup>
  );
}
