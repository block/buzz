import {
  GitBranch,
  GitPullRequest,
  ListTodo,
  MessagesSquare,
  type LucideIcon,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useCommunities } from "@/features/communities/useCommunities";
import type { Channel } from "@/shared/api/types";
import { Switch } from "@/shared/ui/switch";

import type { ChannelProjectFeature } from "../channelProjectFeatures";
import { useChannelProjectFeatures } from "../useChannelProjectFeatures";
import { useCreateProjectMutation } from "../useCreateProject";
import { FieldGroup } from "@/features/channels/ui/ChannelManagementSheetRows";

const FEATURES: Array<{
  feature: ChannelProjectFeature;
  icon: LucideIcon;
  label: string;
}> = [
  { feature: "tasks", icon: ListTodo, label: "Tasks" },
  { feature: "breakouts", icon: MessagesSquare, label: "Breakout channels" },
  { feature: "reviews", icon: GitPullRequest, label: "Reviews" },
  { feature: "repositories", icon: GitBranch, label: "Related repositories" },
];

export function ChannelProjectFeaturesSettings({
  channel,
  currentPubkey,
}: {
  channel: Channel;
  currentPubkey?: string;
}) {
  const { activeCommunity } = useCommunities();
  const context = useChannelProjectFeatures({
    channel,
    currentPubkey,
    relayUrl: activeCommunity?.relayUrl,
  });
  const createProjectMutation = useCreateProjectMutation();
  const [pendingFeature, setPendingFeature] =
    React.useState<ChannelProjectFeature | null>(null);

  async function ensureProject() {
    if (context.project) return context.project;
    const input = {
      description: channel.description,
      homeChannel: channel,
      name: channel.name,
    };
    try {
      return (await createProjectMutation.mutateAsync(input)).project;
    } catch (error) {
      if (
        !(error instanceof Error) ||
        !/already have a project/i.test(error.message)
      ) {
        throw error;
      }
      return (
        await createProjectMutation.mutateAsync({
          ...input,
          name: `${channel.name} ${channel.id.slice(0, 8)}`,
        })
      ).project;
    }
  }

  async function handleFeatureChange(
    feature: ChannelProjectFeature,
    checked: boolean,
  ) {
    setPendingFeature(feature);
    try {
      if (checked) await ensureProject();
      context.setFeatureEnabled(feature, checked);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not update channel features.",
      );
    } finally {
      setPendingFeature(null);
    }
  }

  if (!currentPubkey || !activeCommunity?.relayUrl) return null;

  return (
    <FieldGroup testId="channel-project-features" title="Features">
      {FEATURES.map(({ feature, icon: Icon, label }) => {
        const forcedOn = context.existing[feature];
        const labelId = `channel-feature-${feature}-label`;
        return (
          <div
            className="flex min-h-14 w-full items-center gap-3 px-4 py-3"
            data-testid={`channel-feature-${feature}`}
            key={feature}
          >
            <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span
              className="min-w-0 flex-1 text-sm font-medium text-foreground"
              id={labelId}
            >
              {label}
            </span>
            <Switch
              aria-labelledby={labelId}
              checked={context.enabled[feature]}
              data-testid={`channel-feature-${feature}-switch`}
              disabled={
                forcedOn ||
                pendingFeature !== null ||
                context.projectsQuery.isPending
              }
              onCheckedChange={(checked) => {
                void handleFeatureChange(feature, checked);
              }}
            />
          </div>
        );
      })}
    </FieldGroup>
  );
}
