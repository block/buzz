import type { ChannelFileEntry } from "@/shared/api/channelFiles";

export type FileVersionChain = {
  /** The current version — nothing in this channel supersedes it. */
  latest: ChannelFileEntry;
  /** Every prior version behind `latest`, newest-first. Empty when unversioned. */
  older: ChannelFileEntry[];
};

export function resolveLatestEventId(
  startId: string,
  supersededByEventId: Map<string, string>,
): string;

export function buildFileVersionChains(
  files: ChannelFileEntry[] | null | undefined,
): FileVersionChain[];

export function buildLatestVersionIndex(
  files: ChannelFileEntry[] | null | undefined,
): Map<string, string>;
