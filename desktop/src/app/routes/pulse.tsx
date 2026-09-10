import * as React from "react";
import { parseProjectDetailSearch } from "@/features/projects/lib/projectDetailSearch";
import {
  parseWorkflowEditorPane,
  serializeWorkflowEditorPane,
} from "@/features/workflows/ui/workflowEditorPane";
import { createFileRoute } from "@tanstack/react-router";

import {
  parseProfilePanelTab,
  parseProfilePanelView,
  type ProfilePanelTab,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const PulseScreen = React.lazy(async () => {
  const module = await import("@/features/pulse/ui/PulseScreen");
  return { default: module.PulseScreen };
});

type PulseRouteSearch = ReturnType<typeof parseProjectDetailSearch> & {
  projectId?: string;
  workflowId?: string;
  profilePersona?: string;
  view?: "create" | "edit" | "duplicate";
  pane?: string;
  feed?: string;
  layout?: "combined" | "separate";
  conversation?: string;
  dm?: string;
  channel?: string;
  post?: string;
  reply?: string;
  thread?: string;
  agentSession?: string;
  agentSessionChannel?: string;
  channelManagement?: string;
  profile?: string;
  profileTab?: ProfilePanelTab;
  profileView?: ProfilePanelView;
};

function validatePulseSearch(
  search: Record<string, unknown>,
): PulseRouteSearch {
  const stringValue = (key: string) =>
    typeof search[key] === "string" && search[key].length > 0
      ? search[key]
      : undefined;
  return {
    ...parseProjectDetailSearch(search),
    projectId: stringValue("projectId"),
    workflowId: stringValue("workflowId"),
    profilePersona: stringValue("profilePersona"),
    view:
      search.view === "create" ||
      search.view === "edit" ||
      search.view === "duplicate"
        ? search.view
        : undefined,
    pane: serializeWorkflowEditorPane(parseWorkflowEditorPane(search.pane)),
    feed: [
      "search",
      "dm",
      "channel",
      "agent",
      "conversation",
      "projects",
      "agents",
      "workflows",
    ].includes(String(search.feed))
      ? String(search.feed)
      : undefined,
    layout:
      search.layout === "separate"
        ? "separate"
        : search.layout === "combined"
          ? "combined"
          : undefined,
    conversation: stringValue("conversation"),
    dm: stringValue("dm"),
    channel: stringValue("channel"),
    post: stringValue("post"),
    reply: stringValue("reply"),
    thread: stringValue("thread"),
    agentSession: stringValue("agentSession"),
    agentSessionChannel: stringValue("agentSessionChannel"),
    channelManagement: stringValue("channelManagement"),
    profile:
      typeof search.profile === "string" && search.profile.length > 0
        ? search.profile
        : undefined,
    profileTab: parseProfilePanelTab(search.profileTab) ?? undefined,
    profileView: parseProfilePanelView(search.profileView) ?? undefined,
  };
}

export const Route = createFileRoute("/pulse")({
  validateSearch: validatePulseSearch,
  component: PulseRouteComponent,
});

function PulseRouteComponent() {
  usePreviewFeatureWarning("pulse");
  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="pulse" />}
    >
      <PulseScreen />
    </React.Suspense>
  );
}
