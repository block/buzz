export type MessagesDestination =
  | { type: "empty" }
  | { type: "channel"; channelId: string }
  | { type: "new"; originChannelId: string }
  | { type: "session"; channelId: string };

export type NavigationDestination = "channels" | "agents";

export type NavigableChannel = {
  id: string;
};
