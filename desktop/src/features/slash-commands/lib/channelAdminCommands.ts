import { SlashUsageError } from "./slashUsageError";
import type { SlashCommandDefinition } from "./types";

export const TOGGLE_CHANNEL_MUTE_UI = "toggle-channel-mute";

export type ToggleChannelMutePayload = {
  channelId: string;
  /** Called synchronously by the handler with the mute state it applied. */
  report: (muted: boolean) => void;
};

export type ChannelAdminActions = {
  canManageChannel: boolean;
  canSetTopic: boolean;
  renameChannel: (name: string) => Promise<unknown>;
  archiveChannel: () => Promise<unknown>;
  setTopic: (topic: string) => Promise<unknown>;
};

/** Trims `name` and strips any leading `#` typed as a channel reference. */
export function canonicalChannelName(name: string): string {
  return name.trim().replace(/^#+/, "").trim();
}

function channelLabel(name: string, isDm: boolean): string {
  const trimmed = name.trim();
  if (isDm) return trimmed || "this conversation";
  return trimmed ? `#${trimmed}` : "this channel";
}

export function buildChannelAdminCommands(
  actions: ChannelAdminActions,
): SlashCommandDefinition[] {
  return [
    {
      name: "topic",
      argumentHint: "<text>",
      description: "Set the channel topic",
      usage: "/topic <text>",
      context: "channel",
      isVisible: () => actions.canSetTopic,
      run: async ({ args, toast }) => {
        const topic = args.trim();
        if (!topic) throw new SlashUsageError();
        await actions.setTopic(topic);
        toast.success("Topic set");
      },
    },
    {
      name: "rename",
      argumentHint: "<name>",
      description: "Rename this channel",
      usage: "/rename <name>",
      context: "channel",
      isVisible: () => actions.canManageChannel,
      run: async ({ args, channelName, toast }) => {
        const name = canonicalChannelName(args);
        if (!name) throw new SlashUsageError();
        if (name === channelName.trim()) {
          throw new Error(`This channel is already named #${name}`);
        }
        await actions.renameChannel(name);
        toast.success(`Renamed to #${name}`);
      },
    },
    {
      name: "archive",
      description: "Archive this channel",
      usage: "/archive",
      context: "channel",
      isVisible: () => actions.canManageChannel,
      run: async ({ channelName, toast }) => {
        await actions.archiveChannel();
        toast.success(`Archived ${channelLabel(channelName, false)}`);
      },
    },
    {
      name: "mute",
      description: "Mute or unmute this channel",
      usage: "/mute",
      context: "any",
      run: ({ channelId, channelName, isDm, requestUi, toast }) => {
        let muted: boolean | null = null;
        const payload: ToggleChannelMutePayload = {
          channelId,
          report: (next) => {
            muted = next;
          },
        };
        const handled = requestUi(TOGGLE_CHANNEL_MUTE_UI, payload);
        if (!handled || muted === null) {
          throw new Error("Couldn't change the mute setting. Try again.");
        }
        const label = channelLabel(channelName, isDm);
        toast.success(muted ? `Muted ${label}` : `Unmuted ${label}`);
      },
    },
  ];
}
