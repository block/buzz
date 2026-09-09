export const PULSE_CONVERSATION_KEYS = [
  "dm",
  "conversation",
  "channel",
  "post",
  "reply",
  "thread",
  "profile",
  "profileTab",
  "profileView",
  "agentSession",
  "agentSessionChannel",
  "channelManagement",
] as const;

export const CLEAR_CONVERSATION_PANELS = {
  post: null,
  reply: null,
  thread: null,
  profile: null,
  profileTab: null,
  profileView: null,
  agentSession: null,
  agentSessionChannel: null,
  channelManagement: null,
} as const;
