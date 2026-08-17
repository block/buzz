/**
 * The line shown while an update downloads.
 *
 * Pure and dependency-free, in a `.mjs` sibling so `node:test` exercises the
 * exact source the UI runs (same rationale as `applyEditTagOverlay.mjs`).
 *
 * Why this exists at all: the settings row used to read "Downloading
 * update..." for the whole download and never changed, which is
 * indistinguishable from a stall. On a slow connection people concluded the
 * updater was broken.
 *
 * `totalBytes` is null whenever the server sends no Content-Length — Tauri
 * only reports a total in its `Started` event, and cannot invent one. That
 * case still gets a useful line ("Downloading update — 8.2 MB") rather than a
 * percentage the app cannot honestly compute.
 */

const BYTES_PER_MB = 1024 * 1024;

/** Megabytes to one decimal, which is the right precision for a ~50 MB file. */
function toMegabytes(bytes) {
  return (Math.max(0, bytes ?? 0) / BYTES_PER_MB).toFixed(1);
}

export function formatDownloadProgress(downloadedBytes, totalBytes) {
  const downloaded = toMegabytes(downloadedBytes);

  if (typeof totalBytes !== "number" || totalBytes <= 0) {
    return `Downloading update — ${downloaded} MB`;
  }

  // Clamped because the final chunk can overshoot a slightly stale total, and
  // "101%" undermines confidence in everything else on the page.
  const percent = Math.min(
    100,
    Math.round((Math.max(0, downloadedBytes ?? 0) / totalBytes) * 100),
  );
  return `Downloading update — ${downloaded} MB of ${toMegabytes(
    totalBytes,
  )} MB (${percent}%)`;
}
