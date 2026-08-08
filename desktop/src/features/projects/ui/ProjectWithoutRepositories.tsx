import type * as React from "react";

import type { Project } from "@/features/projects/hooks";
import type { ProjectConnectionScope } from "@/shared/api/projectConnectionTypes";
import { ProjectConnectionScopeUnavailable } from "./ProjectConnectionScopeUnavailable";
import { ProjectConnectionsPanel } from "./ProjectConnectionsPanel";
import { ProjectDetailChrome } from "./ProjectDetailChrome";
import { UnavailableProjectRepositories } from "./UnavailableProjectRepositories";

export function ProjectWithoutRepositories({
  chromeRef,
  connectionScope,
  connectionScopeLoading,
  onGoChannel,
  onGoProjects,
  project,
}: {
  chromeRef: React.Ref<HTMLDivElement>;
  connectionScope: ProjectConnectionScope | null;
  connectionScopeLoading: boolean;
  onGoChannel: (channelId: string) => void;
  onGoProjects: () => void;
  project: Project;
}) {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <ProjectDetailChrome
        activeTabCrumb={null}
        activeWorkItemCrumb={null}
        chromeRef={chromeRef}
        onGoChannel={onGoChannel}
        onGoProjectHome={onGoProjects}
        onGoProjects={onGoProjects}
        project={project}
      />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4">
        <div className="w-full space-y-3 pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1px)]">
          <div className="space-y-0.5">
            <h2 className="truncate text-xl font-semibold tracking-tight">
              {project.name}
            </h2>
            <p className="text-sm text-muted-foreground">
              No repositories are available yet. Project connections still work
              and will apply when repositories are added.
            </p>
          </div>
          {connectionScope ? (
            <ProjectConnectionsPanel projectScope={connectionScope} />
          ) : (
            <ProjectConnectionScopeUnavailable
              loading={connectionScopeLoading}
            />
          )}
          <UnavailableProjectRepositories project={project} />
        </div>
      </div>
    </div>
  );
}
