import type { ChannelSuggestion } from "@/features/messages/lib/useChannelLinks";
import { ChannelAutocomplete } from "@/features/messages/ui/ChannelAutocomplete";
import {
  MentionAutocomplete,
  type MentionSuggestion,
} from "@/features/messages/ui/MentionAutocomplete";

type ForumComposerAutocompletesProps = {
  channelSelectedIndex: number;
  channelSuggestions: ChannelSuggestion[];
  composerOwnsFocus: boolean;
  mentionSelectedIndex: number;
  mentionSuggestions: MentionSuggestion[];
  onChannelSelect: (suggestion: ChannelSuggestion) => void;
  isMentionOpen: boolean;
  isMentionLoading: boolean;
  onMentionDismiss: () => void;
  onMentionSelect: (suggestion: MentionSuggestion) => void;
  position: "above" | "below";
};

export function ForumComposerAutocompletes({
  channelSelectedIndex,
  channelSuggestions,
  composerOwnsFocus,
  mentionSelectedIndex,
  mentionSuggestions,
  onChannelSelect,
  isMentionOpen,
  isMentionLoading,
  onMentionDismiss,
  onMentionSelect,
  position,
}: ForumComposerAutocompletesProps) {
  return (
    <>
      <ChannelAutocomplete
        composerOwnsFocus={composerOwnsFocus}
        onSelect={onChannelSelect}
        position={position}
        selectedIndex={channelSelectedIndex}
        suggestions={channelSuggestions}
      />
      <MentionAutocomplete
        composerOwnsFocus={composerOwnsFocus}
        onDismiss={onMentionDismiss}
        isOpen={isMentionOpen}
        isLoading={isMentionLoading}
        onSelect={onMentionSelect}
        position={position}
        selectedIndex={mentionSelectedIndex}
        suggestions={mentionSuggestions}
      />
    </>
  );
}
