import * as React from "react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Project } from "./projectModels";

/** Load relay-backed ownership profiles for project deletion capability. */
export function useProjectOwnerProfiles(projects: Project[]) {
  const ownerPubkeys = React.useMemo(
    () => [...new Set(projects.map((project) => project.owner))],
    [projects],
  );
  return useUsersBatchQuery(ownerPubkeys, { enabled: ownerPubkeys.length > 0 })
    .data?.profiles;
}
