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
      contentClassName="px-2"
      count={count}
      groupKey={label}
      headerTestId="project-work-item-group-header"
      icon={icon}
      items={items}
      label={label}
      testId="project-work-item-group"
    >
      {children}
    </ProjectSelectableGroup>
  );
}
