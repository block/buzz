/**
 * Shared Appearance scale ladder for interface, chat text, and avatar size.
 * Values are unitless multipliers (1 = 100%).
 */
export const APPEARANCE_SCALE_PRESETS = [
  0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5,
] as const;

export type AppearanceScalePreset = (typeof APPEARANCE_SCALE_PRESETS)[number];

/**
 * Soft guidance only: interface scale above this may make chrome hard to use.
 * The full ladder (including 500%) remains available.
 */
export const INTERFACE_SCALE_SOFT_WARN_THRESHOLD = 2;

export type AppearanceNamedPresetId = "compact" | "comfortable" | "large";

export type AppearanceNamedPresetScales = {
  interface: number;
  chat: number;
  avatar: number;
};

/**
 * Named Appearance bundles. All values are members of
 * {@link APPEARANCE_SCALE_PRESETS} so setters snap without drift.
 */
export const APPEARANCE_NAMED_PRESETS: Record<
  AppearanceNamedPresetId,
  AppearanceNamedPresetScales
> = {
  compact: { interface: 0.9, chat: 0.9, avatar: 0.9 },
  comfortable: { interface: 1, chat: 1, avatar: 1 },
  large: { interface: 1.25, chat: 1.25, avatar: 1.25 },
};

export const APPEARANCE_NAMED_PRESET_ORDER = [
  "compact",
  "comfortable",
  "large",
] as const satisfies readonly AppearanceNamedPresetId[];

export const APPEARANCE_NAMED_PRESET_LABELS: Record<
  AppearanceNamedPresetId,
  string
> = {
  compact: "Compact",
  comfortable: "Comfortable",
  large: "Large",
};

export function isAppearanceNamedPresetActive(
  id: AppearanceNamedPresetId,
  scales: AppearanceNamedPresetScales,
): boolean {
  const preset = APPEARANCE_NAMED_PRESETS[id];
  return (
    scales.interface === preset.interface &&
    scales.chat === preset.chat &&
    scales.avatar === preset.avatar
  );
}
