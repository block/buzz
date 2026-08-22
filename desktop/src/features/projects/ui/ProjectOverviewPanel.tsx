import type { ProjectRepoFile } from "@/features/projects/hooks";
import type { ProjectRepoUnavailableReason } from "@/features/projects/lib/projectRepoAvailability";
import type * as React from "react";
import { cn } from "@/shared/lib/cn";
import { ReadmePanel } from "./ProjectReadmePanel";
import type { RepositoryFileContentSource } from "./useRepositoryFileContent";
import type { RepoSourceHeaderControls } from "./ProjectRepositorySource";

type ProjectOverviewPanelProps = {
  /** `buzz-channel` binding of the repository, for access-restricted copy. */
  accessChannelId?: string | null;
  externalHost?: string;
  externalUrl?: string | null;
  fileContentSource?: RepositoryFileContentSource;
  gitDataState: GitDataState;
  /** Hide the readme header rows when the workspace renders them itself. */
  hideReadmeHeader?: boolean;
  ownerAvatarUrl?: string | null;
  ownerIsAgent?: boolean;
  ownerName?: string;
  readmeFile: ProjectRepoFile | null;
  /** Branch picker + remote/local toggle for the readme header. */
  sourceControls?: RepoSourceHeaderControls;
  unavailableReason?: ProjectRepoUnavailableReason;
};

export type GitDataState = "checking" | "available" | "empty" | "unavailable";

export function OverviewRailSection({
  children,
  title,
  titleClassName,
}: {
  children: React.ReactNode;
  title: string;
  titleClassName?: string;
}) {
  return (
    <section className="space-y-2">
      <h3
        className={cn("text-sm font-semibold text-foreground", titleClassName)}
      >
        {title}
      </h3>
      {children}
    </section>
  );
}

export function ProjectOverviewPanel({
  accessChannelId,
  externalHost,
  externalUrl,
  fileContentSource,
  gitDataState,
  hideReadmeHeader,
  ownerAvatarUrl,
  ownerIsAgent,
  ownerName,
  readmeFile,
  sourceControls,
  unavailableReason,
}: ProjectOverviewPanelProps) {
  return (
    <div className="min-w-0">
      {/* ReadmePanel renders its own "no README" fallback while keeping
          repository recovery actions reachable. */}
      <ReadmePanel
        accessChannelId={accessChannelId}
        externalHost={externalHost}
        externalUrl={externalUrl}
        file={readmeFile}
        fileContentSource={fileContentSource}
        gitDataState={gitDataState}
        hideHeader={hideReadmeHeader}
        ownerAvatarUrl={ownerAvatarUrl}
        ownerIsAgent={ownerIsAgent}
        ownerName={ownerName}
        sourceControls={sourceControls}
        unavailableReason={unavailableReason}
      />
    </div>
  );
}
