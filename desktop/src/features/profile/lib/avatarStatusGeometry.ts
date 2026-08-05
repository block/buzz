export type AvatarStatusGeometryRatios = {
  /** Horizontal center of the status badge as a fraction of avatar size (0–1). */
  centerX: number;
  /** Vertical center of the status badge as a fraction of avatar size (0–1). */
  centerY: number;
  /** Cutout diameter as a fraction of avatar size (0–1), before absolute clamp. */
  cutoutDiameter: number;
  /** Visible status-dot diameter as a fraction of avatar size (0–1), before clamp. */
  dotDiameter: number;
};

export type AvatarBadgeCircle = {
  cx: number;
  cy: number;
  r: number;
};

export type AvatarBadgeBox = {
  bottom: number;
  height: number;
  right: number;
  width: number;
};

/**
 * Absolute floor/ceiling for presence pips. Linear ratio-only sizing blows up
 * on large heroes / 200–500% avatar scale (a 30% pip on a 96px disc is ~29px).
 * Clamp keeps the indicator readable on 24px DM avatars without dominating
 * profile heroes.
 */
export const MIN_STATUS_DOT_PX = 8;
export const MAX_STATUS_DOT_PX = 12;
export const MIN_STATUS_CUTOUT_PX = 10;
export const MAX_STATUS_CUTOUT_PX = 16;

/**
 * Resolve cutout + badge box for a presence indicator.
 * Preferred size follows ratios, then is clamped so status stays a small pip.
 */
export function resolveAvatarStatusGeometry(
  size: number,
  ratios: AvatarStatusGeometryRatios,
): {
  cutout: AvatarBadgeCircle;
  badgeBox: AvatarBadgeBox;
} {
  const preferredDot = size * ratios.dotDiameter;
  const dotSize = Math.min(
    MAX_STATUS_DOT_PX,
    Math.max(MIN_STATUS_DOT_PX, preferredDot),
  );

  // Keep cutout a bit larger than the visible dot (mask ring), also clamped.
  const cutoutToDot =
    ratios.dotDiameter > 0 ? ratios.cutoutDiameter / ratios.dotDiameter : 1.4;
  const preferredCutout = Math.max(
    size * ratios.cutoutDiameter,
    dotSize * cutoutToDot,
  );
  const cutoutDiameter = Math.min(
    MAX_STATUS_CUTOUT_PX,
    Math.max(MIN_STATUS_CUTOUT_PX, preferredCutout, dotSize * 1.25),
  );

  // Anchor near the lower-right rim. When the pip is clamped smaller than the
  // ratio would imply, pull the center slightly outward so the notch stays on
  // the rim rather than floating inward.
  const rimInset = cutoutDiameter / 2;
  const centerX = Math.min(
    size - rimInset,
    Math.max(rimInset, size * ratios.centerX),
  );
  const centerY = Math.min(
    size - rimInset,
    Math.max(rimInset, size * ratios.centerY),
  );

  return {
    cutout: {
      cx: centerX,
      cy: centerY,
      r: cutoutDiameter / 2,
    },
    badgeBox: {
      bottom: size - centerY - dotSize / 2,
      height: dotSize,
      right: size - centerX - dotSize / 2,
      width: dotSize,
    },
  };
}

/**
 * Hover profile card. Ratios approximate the historic 40px geometry
 * (10px dot / 16px cutout) and clamp keeps large cards from bloating.
 */
export const HOVER_PROFILE_STATUS_RATIOS = {
  centerX: 0.85,
  centerY: 0.85,
  cutoutDiameter: 0.32,
  dotDiameter: 0.2,
} as const satisfies AvatarStatusGeometryRatios;

/**
 * Profile panel hero. Same pip language as hover; clamp caps absolute size on
 * 96px+ heroes so the status does not read as a second avatar.
 */
export const PROFILE_HERO_STATUS_RATIOS = {
  centerX: 0.85,
  centerY: 0.85,
  cutoutDiameter: 0.22,
  dotDiameter: 0.14,
} as const satisfies AvatarStatusGeometryRatios;
