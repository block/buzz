import type { ChannelFileEntry } from "@/shared/api/channelFiles";

export const SUPERSEDES_MATCH_WINDOW_SECONDS: number;
export const SUPERSEDES_PRESELECT_SCORE: number;

export type RankedSupersedesCandidate = {
  file: ChannelFileEntry;
  /** 0–100; see `scoreFilenameSimilarity`. */
  score: number;
};

export function normalizeStem(stem: string): string;

export function scoreFilenameSimilarity(
  uploadFilename: string,
  candidateFilename: string,
): number;

export function rankSupersedesCandidates(
  uploadFilename: string | null | undefined,
  uploadSha256: string | null | undefined,
  files: ChannelFileEntry[] | null | undefined,
  nowSeconds?: number,
): RankedSupersedesCandidate[];

export function preselectedSupersedesCandidate(
  ranked: RankedSupersedesCandidate[],
): ChannelFileEntry | null;
