export function formatTimecode(
  seconds: number,
  options: { fractionalDigits?: number; trimZeroFraction?: boolean } = {},
): string {
  const safeSeconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  const fractionalDigits = Math.max(0, options.fractionalDigits ?? 0);
  const precisionFactor = 10 ** fractionalDigits;
  const totalTicks =
    fractionalDigits > 0
      ? Math.round(safeSeconds * precisionFactor)
      : Math.floor(safeSeconds);
  const totalSeconds =
    fractionalDigits > 0
      ? Math.floor(totalTicks / precisionFactor)
      : totalTicks;
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const remainingSeconds = totalSeconds % 60;
  const paddedSeconds = String(remainingSeconds).padStart(2, "0");
  const fraction = fractionalDigits > 0 ? totalTicks % precisionFactor : 0;

  const baseTimecode =
    hours > 0
      ? `${hours}:${String(minutes).padStart(2, "0")}:${paddedSeconds}`
      : `${String(minutes).padStart(2, "0")}:${paddedSeconds}`;

  if (
    fractionalDigits <= 0 ||
    (options.trimZeroFraction === true && fraction === 0)
  ) {
    return baseTimecode;
  }

  return `${baseTimecode}.${String(fraction).padStart(fractionalDigits, "0")}`;
}

export function formatCommentTimecode(seconds: number): string {
  return formatTimecode(seconds, {
    fractionalDigits: 1,
    trimZeroFraction: true,
  });
}
