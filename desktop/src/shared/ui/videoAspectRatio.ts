import * as React from "react";

// Natural aspect ratios of videos that arrived without a NIP-92 `dim` tag,
// keyed by resolved URL and learned from the first `loadedmetadata`. Component
// state is lost when the virtualized timeline evicts and later remounts a row;
// without this module-level memory a dim-less video would remount at the 16/9
// fallback and then resize to its true ratio when metadata re-loads — a
// remount height mismatch that triggers the virtualizer's jump compensation.
// Same pattern as `decodedImageDimensions` in markdown/utils.ts.
const learnedVideoAspectRatios = new Map<string, number>();

export function rememberVideoAspectRatio(
  src: string | undefined,
  ratio: number,
): void {
  if (!src || !Number.isFinite(ratio) || ratio <= 0) return;
  learnedVideoAspectRatios.set(src, ratio);
}

export function getRememberedVideoAspectRatio(
  src: string | undefined,
): number | null {
  return (src ? learnedVideoAspectRatios.get(src) : undefined) ?? null;
}

/**
 * A video's measured natural aspect ratio, seeded from the module cache so a
 * remount of the same `src` starts at the ratio learned before eviction
 * instead of `null` (which the player renders as the 16/9 placeholder).
 */
export function useNaturalVideoAspectRatio(
  src: string,
): [number | null, (width: number, height: number) => void] {
  const [ratio, setRatio] = React.useState<number | null>(() =>
    getRememberedVideoAspectRatio(src),
  );
  const learn = React.useCallback(
    (width: number, height: number) => {
      if (width <= 0 || height <= 0) return;
      const next = width / height;
      rememberVideoAspectRatio(src, next);
      setRatio(next);
    },
    [src],
  );
  return [ratio, learn];
}
