import type { ChannelSuggestion } from "@/features/messages/lib/useChannelLinks";
import { ChannelAutocomplete } from "@/features/messages/ui/ChannelAutocomplete";
import {
  MentionAutocomplete,
  type MentionSuggestion,
} from "@/features/messages/ui/MentionAutocomplete";

type ForumComposerAutocompletesProps = {
  channelSelectedIndex: number;
  channelSuggestions: ChannelSuggestion[];
  isEditorFocused: boolean;
  mentionSelectedIndex: number;
  mentionSuggestions: MentionSuggestion[];
  onChannelSelect: (suggestion: ChannelSuggestion) => void;
  onMentionFetchMore?: () => void;
  onMentionDismiss: () => void;
  onMentionSelect: (suggestion: MentionSuggestion) => void;
  position: "above" | "below";
};

export function ForumComposerAutocompletes({
  channelSelectedIndex,
  channelSuggestions,
  isEditorFocused,
  mentionSelectedIndex,
  mentionSuggestions,
  onChannelSelect,
  onMentionFetchMore,
  onMentionDismiss,
  onMentionSelect,
  position,
}: ForumComposerAutocompletesProps) {
  return (
    <>
      <ChannelAutocomplete
        isEditorFocused={isEditorFocused}
        onSelect={onChannelSelect}
        position={position}
        selectedIndex={channelSelectedIndex}
        suggestions={channelSuggestions}
      />
      <MentionAutocomplete
        isEditorFocused={isEditorFocused}
        onDismiss={onMentionDismiss}
        onFetchMore={onMentionFetchMore}
        onSelect={onMentionSelect}
        position={position}
        selectedIndex={mentionSelectedIndex}
        suggestions={mentionSuggestions}
      />
    </>
  );
}
