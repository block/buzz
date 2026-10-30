import type { RespondToMode } from "./types";

export type RelayMemberRole = "owner" | "admin" | "member";

export type RelayMember = {
  pubkey: string;
  role: RelayMemberRole;
  addedBy: string | null;
  createdAt: string;
};

export type RelayAgent = {
  pubkey: string;
  ownerPubkey: string | null;
  name: string;
  agentType: string;
  channels: string[];
  channelIds: string[];
  capabilities: string[];
  /** Policy-only discovery has no liveness evidence. */
  status: "online" | "away" | "offline" | "unknown";
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
  /** Opaque id of the device that holds this agent's secret. */
  deviceId: string | null;
  /** Human label for that device, or null on pre-feature events. */
  deviceLabel: string | null;
};

/** Identity of the computer this Buzz install runs on. */
export type DeviceIdentity = {
  deviceId: string;
  deviceLabel: string;
  createdAt: string;
};
