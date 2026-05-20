import type { ChannelSuggestion } from "@/features/messages/lib/useChannelLinks";
import { ChannelAutocomplete } from "@/features/messages/ui/ChannelAutocomplete";
import {
  MentionAutocomplete,
  type MentionSuggestion,
} from "@/features/messages/ui/MentionAutocomplete";

type ForumComposerAutocompletesProps = {
  channelSelectedIndex: number;
  channelSuggestions: ChannelSuggestion[];
  mentionSelectedIndex: number;
  mentionSuggestions: MentionSuggestion[];
  onChannelSelect: (suggestion: ChannelSuggestion) => void;
  onMentionSelect: (suggestion: MentionSuggestion) => void;
  position: "above" | "below";
};

export function ForumComposerAutocompletes({
  channelSelectedIndex,
  channelSuggestions,
  mentionSelectedIndex,
  mentionSuggestions,
  onChannelSelect,
  onMentionSelect,
  position,
}: ForumComposerAutocompletesProps) {
  return (
    <>
      <ChannelAutocomplete
        onSelect={onChannelSelect}
        position={position}
        selectedIndex={channelSelectedIndex}
        suggestions={channelSuggestions}
      />
      <MentionAutocomplete
        onSelect={onMentionSelect}
        position={position}
        selectedIndex={mentionSelectedIndex}
        suggestions={mentionSuggestions}
      />
    </>
  );
}
