import type * as React from "react";

import {
  ACCENT_COLORS,
  DEFAULT_ACCENT,
  resolveColonyAccentPalette,
} from "@/shared/theme/colonyAccentPalette";
import {
  DEFAULT_FIELD_COLORS,
  type FieldColors,
} from "@/shared/theme/sidebarField";

import type { AppearanceSnapshot } from "../lib/appearanceSnapshot";
import { conversationMessageSizeCss } from "../lib/conversationMessageSizePreference";

/** The six accents the Colony 3.0 design offers (Settings, Appearance). */
export const ACCENTS = ACCENT_COLORS.map(
  ({ name, value }) => [name, value] as const,
);

/**
 * The preview card on the Appearance page: a sidebar drawn from the person's
 * three field colours next to one channel message, with the accent on its
 * action. The Apply bar sits under it (see the panel).
 */
export function LiveAppearancePreview({
  preferences,
  isDark,
}: {
  preferences: AppearanceSnapshot;
  isDark: boolean;
}) {
  const field: FieldColors = preferences.field ?? [...DEFAULT_FIELD_COLORS];
  const accent =
    ACCENT_COLORS.find(
      ({ value }) => value.toLowerCase() === preferences.accent.toLowerCase(),
    )?.value ?? preferences.accent;
  const previewAccent = isDark
    ? accent.toLowerCase() === DEFAULT_ACCENT.toLowerCase()
      ? "hsl(var(--colony-accent))"
      : resolveColonyAccentPalette(accent, "dark").accentHex
    : accent;
  return (
    <div
      className={isDark ? "cs-preview dark" : "cs-preview"}
      data-testid="appearance-live-preview"
      style={
        {
          "--ap-field-top": field[0],
          "--ap-field-middle": field[1],
          "--ap-field-bottom": field[2],
          "--cs-preview-accent": previewAccent,
          "--cs-preview-message": conversationMessageSizeCss(
            preferences.messageSize,
          ),
        } as React.CSSProperties
      }
    >
      <div aria-hidden="true" className="cs-preview-nav">
        <i className="cs-preview-blob cs-preview-blob-top" />
        <i className="cs-preview-blob cs-preview-blob-middle" />
        <i className="cs-preview-blob cs-preview-blob-bottom" />
        <i className="cs-preview-veil" />
        <span className="cs-preview-lines">
          <b />
          <u />
          <s />
          <s />
          <s />
        </span>
      </div>
      <div aria-hidden="true" className="cs-preview-chat">
        <strong># Client workspace</strong>
        <div className="cs-preview-message">
          <span className="cs-preview-avatar">M</span>
          <span>
            <small>Team member</small>
            <span>The next post is ready for your review.</span>
          </span>
        </div>
        <span className="cs-preview-action">Approve</span>
      </div>
    </div>
  );
}
