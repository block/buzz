import { APPEARANCE_SCALE_PRESETS } from "./appearanceScalePresets";
import { createScalePreference } from "./scalePreference";

/**
 * Message / chat content scale (body + author text in the timeline).
 * Independent of global interface (root rem) scale; relative to it.
 *
 * CSS: `--buzz-chat-scale` on `<html>` (always written, including default 1).
 */

export const chatScalePreference = createScalePreference({
  storageKey: "buzz:chat-scale",
  cssVar: "--buzz-chat-scale",
  presets: APPEARANCE_SCALE_PRESETS,
  // Keep the var present so Tailwind/CSS `calc(... * var(--buzz-chat-scale))`
  // never depends on a missing property or a comma-fallback parse quirk.
  clearCssVarAtDefault: false,
});

export const CHAT_SCALE_STORAGE_KEY = chatScalePreference.STORAGE_KEY;
export const CHAT_SCALE_CSS_VAR = "--buzz-chat-scale";
export const DEFAULT_CHAT_SCALE = chatScalePreference.DEFAULT;
export const MIN_CHAT_SCALE = chatScalePreference.MIN;
export const MAX_CHAT_SCALE = chatScalePreference.MAX;
export const CHAT_SCALE_PRESETS = chatScalePreference.PRESETS;

export const getChatScale = chatScalePreference.get;
export const setChatScale = chatScalePreference.set;
export const applyCurrentChatScale = chatScalePreference.applyCurrent;
export const subscribeChatScale = chatScalePreference.subscribe;
export const getChatScaleSnapshot = chatScalePreference.getSnapshot;
export const getChatScaleServerSnapshot = chatScalePreference.getServerSnapshot;
export const formatChatScalePercent = chatScalePreference.formatPercent;
export const chatScalePresetIndex = chatScalePreference.presetIndex;
export const normalizeChatScale = chatScalePreference.normalize;
