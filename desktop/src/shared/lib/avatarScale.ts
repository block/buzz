import { APPEARANCE_SCALE_PRESETS } from "./appearanceScalePresets";
import { createScalePreference } from "./scalePreference";

/**
 * Identity avatar scale (channels, threads, profiles, side surfaces).
 * Independent of global interface scale and chat text scale; relative to
 * interface scale in Appearance.
 *
 * CSS on `<html>`:
 * - `--buzz-avatar-scale` (unitless factor, always set)
 * - `--buzz-message-avatar-size` (rem length for md avatars, always set)
 */

/** Semantic base sizes at Appearance → Avatar size = 100%. */
export const AVATAR_BASE_SIZE_REM = {
  xs: 1.25,
  sm: 1.5,
  md: 3,
} as const;

export type AvatarSize = keyof typeof AVATAR_BASE_SIZE_REM;

/** @deprecated Prefer {@link AVATAR_BASE_SIZE_REM}.md / {@link getAvatarSizeRem}. */
export const BASE_MESSAGE_AVATAR_SIZE_REM = AVATAR_BASE_SIZE_REM.md;

const base = createScalePreference({
  storageKey: "buzz:avatar-scale",
  cssVar: "--buzz-avatar-scale",
  presets: APPEARANCE_SCALE_PRESETS,
  // Always write the unitless factor so CSS calc() consumers stay reliable.
  clearCssVarAtDefault: false,
});

export const AVATAR_SCALE_STORAGE_KEY = base.STORAGE_KEY;
export const AVATAR_SCALE_CSS_VAR = "--buzz-avatar-scale";
export const DEFAULT_AVATAR_SCALE = base.DEFAULT;
export const MIN_AVATAR_SCALE = base.MIN;
export const MAX_AVATAR_SCALE = base.MAX;
export const AVATAR_SCALE_PRESETS = base.PRESETS;

export const getAvatarScale = base.get;
export const subscribeAvatarScale = base.subscribe;
export const getAvatarScaleSnapshot = base.getSnapshot;
export const getAvatarScaleServerSnapshot = base.getServerSnapshot;
export const formatAvatarScalePercent = base.formatPercent;
export const avatarScalePresetIndex = base.presetIndex;
export const normalizeAvatarScale = base.normalize;

/**
 * Resolved rem for a semantic avatar size at the given (or current) scale.
 */
export function getAvatarSizeRem(
  size: AvatarSize,
  scale: number = getAvatarScale(),
): number {
  return AVATAR_BASE_SIZE_REM[size] * normalizeAvatarScale(scale);
}

/**
 * Avatar size in rem for the default message avatar (48px / 3rem at scale 1).
 */
export function getMessageAvatarSizeRem(scale = getAvatarScale()): number {
  return getAvatarSizeRem("md", scale);
}

/**
 * Apply both CSS variables. Always set (including default) so layout and
 * `var()` consumers stay in lockstep with the React store.
 */
function applyMessageAvatarCssVars(scale = getAvatarScale()): void {
  if (typeof document === "undefined") {
    return;
  }
  const normalized = normalizeAvatarScale(scale);
  const root = document.documentElement.style;
  root.setProperty("--buzz-avatar-scale", String(normalized));
  root.setProperty(
    "--buzz-message-avatar-size",
    `${getMessageAvatarSizeRem(normalized)}rem`,
  );
}

export function setAvatarScale(scale: number): void {
  base.set(scale);
  // Keep --buzz-message-avatar-size in sync after the unitless factor write.
  applyMessageAvatarCssVars(base.get());
}

export function applyCurrentAvatarScale(): void {
  base.applyCurrent();
  applyMessageAvatarCssVars(base.get());
}

// Initial vars (module load).
applyMessageAvatarCssVars(base.get());

export type AvatarSizeStyle = {
  width: string;
  height: string;
  minWidth: string;
  minHeight: string;
  maxWidth: string;
  maxHeight: string;
};

/** @deprecated Prefer {@link AvatarSizeStyle}. */
export type MessageAvatarSizeStyle = AvatarSizeStyle;

/**
 * Inline size styles for identity avatars.
 * Uses a concrete rem value (scale already multiplied) so size updates when
 * React re-renders after the slider changes.
 */
export function avatarSizeStyle(
  size: AvatarSize = "md",
  scale: number = getAvatarScale(),
): AvatarSizeStyle {
  const rem = `${getAvatarSizeRem(size, scale)}rem`;
  return {
    width: rem,
    height: rem,
    minWidth: rem,
    minHeight: rem,
    maxWidth: rem,
    maxHeight: rem,
  };
}

/**
 * Inline size styles for a custom base rem (e.g. DM intro stack 3.75rem).
 */
export function messageAvatarSizeStyle(
  baseSizeRem: number = AVATAR_BASE_SIZE_REM.md,
  scale: number = getAvatarScale(),
): AvatarSizeStyle {
  const size = `${baseSizeRem * normalizeAvatarScale(scale)}rem`;
  return {
    width: size,
    height: size,
    minWidth: size,
    minHeight: size,
    maxWidth: size,
    maxHeight: size,
  };
}
