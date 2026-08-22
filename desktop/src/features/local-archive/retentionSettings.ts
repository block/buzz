// Pure helpers for the observer-frame retention control and the archive size
// readout. Kept free of React and Tauri so the branch-heavy logic — input
// parsing, client-side bound validation, and byte formatting — is unit-tested
// directly, matching the `localArchiveKinds` split in this feature.

import type { ArchiveSizeStats } from "@/shared/api/tauriArchive";

/** Inclusive bounds mirroring the backend's `validate_days` (`retention.rs`). */
export const MIN_RETENTION_DAYS = 1;
export const MAX_RETENTION_DAYS = 36_500;

export type ParsedDays =
  | { ok: true; days: number }
  | { ok: false; error: string };

/**
 * Parse and bound-check a retention-days text input. Accepts only a plain
 * non-negative integer in `[MIN, MAX]`; anything else (blank, non-numeric,
 * fractional, signed, out-of-range) is rejected with a message suitable for
 * inline display. Mirrors the backend contract so the invoke is only issued
 * for values the store will accept.
 */
export function parseRetentionDays(raw: string): ParsedDays {
  const trimmed = raw.trim();
  if (trimmed === "") return { ok: false, error: "Enter a number of days." };
  // Reject signs, decimals, whitespace-in-the-middle — only bare digits.
  if (!/^\d+$/.test(trimmed)) {
    return { ok: false, error: "Days must be a whole number." };
  }
  const days = Number(trimmed);
  if (days < MIN_RETENTION_DAYS) {
    return { ok: false, error: `Days must be at least ${MIN_RETENTION_DAYS}.` };
  }
  if (days > MAX_RETENTION_DAYS) {
    return {
      ok: false,
      error: `Days must be at most ${MAX_RETENTION_DAYS.toLocaleString()}.`,
    };
  }
  return { ok: true, days };
}

/**
 * Human-readable byte size: "0 B", "820 B", "12.4 KB", "3.1 MB", "7.4 GB".
 * Binary (1024) units, matching `FileCard`'s `formatFileSize`. Negative or
 * non-finite inputs clamp to "0 B" rather than rendering nonsense.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = bytes / 1024;
  let i = 0;
  while (size >= 1024 && i < units.length - 1) {
    size /= 1024;
    i += 1;
  }
  return `${size < 10 ? size.toFixed(1) : Math.round(size)} ${units[i]}`;
}

export type ArchiveSizeReadout = {
  /** Physical bytes actually on disk: main DB file + `-wal` sidecar. */
  physicalBytes: number;
  /** Reclaimable bytes sitting on the freelist (`freelistCount × pageSize`). */
  reclaimableBytes: number;
  physicalLabel: string;
  reclaimableLabel: string;
};

/**
 * Derive the size readout from raw stats. Physical size is the honest on-disk
 * footprint (main + WAL); reclaimable is freelist pages, which only shrink the
 * file after an offline VACUUM — until then they are savings, not a smaller
 * file. Callers label them so the UI never implies the file has already shrunk.
 */
export function deriveSizeReadout(stats: ArchiveSizeStats): ArchiveSizeReadout {
  const physicalBytes = stats.mainFileBytes + stats.walFileBytes;
  const reclaimableBytes = Math.max(0, stats.freelistCount * stats.pageSize);
  return {
    physicalBytes,
    reclaimableBytes,
    physicalLabel: formatBytes(physicalBytes),
    reclaimableLabel: formatBytes(reclaimableBytes),
  };
}
