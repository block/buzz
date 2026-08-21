/** A channel content event, reduced to what link collection needs. */
export type LinkSourceMessage = {
  eventId: string;
  pubkey: string;
  /** Unix seconds. */
  createdAt: number;
  content: string;
  /** True when the message also carries an `imeta` attachment of its own. */
  hasAttachment?: boolean;
  /** The event id this message was tagged as superseding, if any. */
  supersedes?: string | null;
};

/**
 * A link entry, shaped to slot into the same list as `ChannelFileEntry`.
 * Kept structural rather than importing that type, so this module stays pure
 * and free of any dependency on the API layer that consumes it.
 */
export type ChannelLinkEntry = {
  kind: "link";
  eventId: string;
  uploadedBy: string;
  uploadedAt: number;
  filename: string | null;
  sha256: null;
  size: null;
  mime: null;
  url: string;
  supersedes: string | null;
  supersededBy: string | null;
};

/** Hostname with any leading `www.` removed, or null if unparseable. */
export function linkHost(url: string): string | null;

/** A human-readable display name for a link, or null if unparseable. */
export function readableLinkName(url: string): string | null;

/** Every distinct http(s) URL in message content, in order of appearance. */
export function extractMessageLinks(content: string): string[];

/** Normalized URL key → the sender's markdown label for that link. */
export function extractMessageLinkLabels(content: string): Map<string, string>;

/** The identity two URLs must share to count as the same link. */
export function normalizeLinkKey(url: string): string | null;

/** One entry per unique link in the channel, dated at its first appearance. */
export function collectChannelLinkEntries(input: {
  messages: readonly LinkSourceMessage[] | undefined;
  excludedUrls: readonly (string | null)[] | undefined;
}): ChannelLinkEntry[];
