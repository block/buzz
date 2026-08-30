import * as React from "react";

import type {
  ChannelSection,
  SmartSectionRule,
} from "@/features/sidebar/lib/channelSectionsStorage";
import type { Channel } from "@/shared/api/types";
import {
  SmartSectionDialog,
  SmartSectionMatchesDialog,
} from "./SmartSectionDialogs";

export function useSmartSectionDialogs({
  sections,
  channels,
  assignments,
  createSection,
  renameSection,
  assignChannels,
}: {
  sections: ChannelSection[];
  channels: Channel[];
  assignments: Record<string, string>;
  createSection: (
    name: string,
    icon?: string,
    smartRule?: SmartSectionRule,
  ) => ChannelSection | null;
  renameSection: (
    sectionId: string,
    name: string,
    icon?: string,
    smartRule?: SmartSectionRule,
  ) => void;
  assignChannels: (channelIds: string[], sectionId: string) => void;
}) {
  const [isCreateOpen, setIsCreateOpen] = React.useState(false);
  const [editTarget, setEditTarget] = React.useState<ChannelSection | null>(
    null,
  );
  const [matchesTarget, setMatchesTarget] =
    React.useState<ChannelSection | null>(null);
  const openCreateDialog = React.useCallback(() => setIsCreateOpen(true), []);

  const dialogs = (
    <>
      <SmartSectionDialog
        onConfirm={(value) => {
          const created = createSection(value.name, undefined, value.smartRule);
          if (created) setIsCreateOpen(false);
        }}
        onOpenChange={setIsCreateOpen}
        open={isCreateOpen}
      />
      <SmartSectionDialog
        onConfirm={(value) => {
          if (!editTarget) return;
          renameSection(
            editTarget.id,
            value.name,
            editTarget.icon,
            value.smartRule,
          );
          setEditTarget(null);
        }}
        onOpenChange={(open) => {
          if (!open) setEditTarget(null);
        }}
        open={editTarget !== null}
        section={editTarget}
      />
      <SmartSectionMatchesDialog
        assignments={assignments}
        channels={channels}
        onConfirm={(channelIds) => {
          if (!matchesTarget) return;
          assignChannels(channelIds, matchesTarget.id);
          setMatchesTarget(null);
        }}
        onOpenChange={(open) => {
          if (!open) setMatchesTarget(null);
        }}
        open={matchesTarget !== null}
        section={matchesTarget}
        sections={sections}
      />
    </>
  );

  return {
    dialogs,
    openCreateDialog,
    openEditDialog: setEditTarget,
    openMatchesDialog: setMatchesTarget,
  };
}
