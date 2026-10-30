import {
  parseProfilePanelTab,
  parseProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { KIND_REPO_ANNOUNCEMENT } from "@/shared/constants/kinds";
import { isEntityLinkTab } from "@/shared/lib/entityLink";

function optionalSearchString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * Project detail URLs carry forge params (repository, tab, issue) and, on
 * channel-first home, the same auxiliary-panel params a stream channel uses
 * (thread, agent session, profile). Both must survive `validateSearch` or
 * ChannelScreen cannot keep threads and agent activity on this route.
 */
export function parseProjectDetailSearch(search: Record<string, unknown>) {
  return {
    commitHash: optionalSearchString(search.commitHash),
    filePath: optionalSearchString(search.filePath),
    pullRequestId: optionalSearchString(search.pullRequestId),
    issueId: optionalSearchString(search.issueId),
    repositoryId: optionalSearchString(search.repositoryId),
    tab: isEntityLinkTab(search.tab) ? search.tab : undefined,
    agentSession: nonEmptyString(search.agentSession),
    agentSessionChannel: nonEmptyString(search.agentSessionChannel),
    autoSend: nonEmptyString(search.autoSend),
    channelManagement: nonEmptyString(search.channelManagement),
    messageId: nonEmptyString(search.messageId),
    profile: nonEmptyString(search.profile),
    profileTab: parseProfilePanelTab(search.profileTab) ?? undefined,
    profileView: parseProfilePanelView(search.profileView) ?? undefined,
    thread: nonEmptyString(search.thread),
    threadRootId: nonEmptyString(search.threadRootId),
  };
}

/**
 * Channel-first project home is the default. A repository forge surface is
 * requested by an explicit repo/work-item search param, or by a legacy
 * kind:30617 project id (the project *is* that repository).
 */
export function wantsProjectRepositorySurface(input: {
  commitHash?: string;
  filePath?: string;
  issueId?: string;
  projectId: string;
  pullRequestId?: string;
  repositoryId?: string;
  tab?: string;
}): boolean {
  if (
    input.repositoryId ||
    input.tab ||
    input.issueId ||
    input.pullRequestId ||
    input.commitHash ||
    input.filePath
  ) {
    return true;
  }
  return input.projectId.startsWith(`${KIND_REPO_ANNOUNCEMENT}:`);
}
