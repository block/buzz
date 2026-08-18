import {
  CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";

export const DM_NOTIFIABLE_EVENT_KINDS = [
  ...CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_HUDDLE_STARTED,
] as const;

const DM_NOTIFIABLE_KINDS = new Set<number>(DM_NOTIFIABLE_EVENT_KINDS);

// DM OS-notifications gate. The DM subscription matches every `h`-tagged
// event in the channel (kind:5/7/9005/edits/etc.), so we must filter to
// human-visible message kinds before firing a toast. Huddle starts are included
// only for DMs because the start card is the invite.
export function isDmNotifiableKind(kind: number): boolean {
  return DM_NOTIFIABLE_KINDS.has(kind);
}
