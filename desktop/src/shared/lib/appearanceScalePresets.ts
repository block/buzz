/**
 * Shared Appearance scale ladder for interface, chat text, and avatar size.
 * Values are unitless multipliers (1 = 100%).
 */
export const APPEARANCE_SCALE_PRESETS = [
  0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5,
] as const;

export type AppearanceScalePreset = (typeof APPEARANCE_SCALE_PRESETS)[number];
