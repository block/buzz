/**
 * EXPLORATION — Discord-style first-open discovery.
 *
 * A compiled directory of communities a brand-new user can join right away,
 * shown on the first-open landing before any identity/agent setup. In a real
 * implementation this list would come from a directory service (or a
 * curated kind:30xxx event on a bootstrap relay); for the exploration it is
 * a compiled constant, exactly like the compiled default relays that
 * `initFirstCommunity` already admits token-less.
 */
export type FeaturedCommunity = {
  /** Stable id for test hooks and keys. */
  id: string;
  name: string;
  tagline: string;
  relayUrl: string;
  /** Approximate member count shown as social proof. */
  members: number;
  /** Accent emoji standing in for a community icon. */
  emoji: string;
  /** True while a relay is open to token-less first connections. */
  openJoin: boolean;
};

export const FEATURED_COMMUNITIES: FeaturedCommunity[] = [
  {
    id: "buzz-hq",
    name: "Buzz HQ",
    tagline: "The team building Buzz — ask anything, meet the agents.",
    relayUrl: "wss://buzz.block.builderlab.xyz",
    members: 128,
    emoji: "🐝",
    openJoin: true,
  },
  {
    id: "agent-builders",
    name: "Agent Builders",
    tagline: "Share agents, adopt the best ones, learn by watching.",
    relayUrl: "wss://agents.buzz.builderlab.xyz",
    members: 342,
    emoji: "🤖",
    openJoin: true,
  },
  {
    id: "nostr-devs",
    name: "Nostr Devs",
    tagline: "Protocol talk, NIPs, relays, and open social infrastructure.",
    relayUrl: "wss://nostr.buzz.builderlab.xyz",
    members: 214,
    emoji: "🟣",
    openJoin: true,
  },
  {
    id: "digital-nomads",
    name: "Digital Nomads",
    tagline: "500 travelers coordinating meetups, visas, and city guides.",
    relayUrl: "wss://nomads.buzz.builderlab.xyz",
    members: 507,
    emoji: "🌍",
    openJoin: true,
  },
];
