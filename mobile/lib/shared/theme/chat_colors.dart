import 'package:flutter/material.dart';

import 'accent_colors.dart';

/// Desktop accent families in picker order. Fills are accent-9 from buzz-app's
/// design-system tokens (4ad00a8a).
///
/// Wire values are desktop's v1 `community-theme` accents for the same color
/// family, so a synced preference means the same color on every client. The
/// cross-client mapping lives in `test-fixtures/community-theme-accents.json`.
const chatColorChoices = [
  AccentColor(
    name: 'Gray',
    light: Color(0xFF525252),
    dark: Color(0xFFC7C7C7),
    wireValue: 'neutral',
  ),
  AccentColor(
    name: 'Chartreuse',
    light: Color(0xFFD7D72E),
    dark: Color(0xFFD7D72E),
    wireValue: '#d7d72e',
  ),
  AccentColor(
    name: 'Flame',
    light: Color(0xFFD93A0C),
    dark: Color(0xFFD93A0C),
    wireValue: '#f97316',
  ),
  AccentColor(
    name: 'Magenta',
    light: Color(0xFFCC1F7A),
    dark: Color(0xFFCC1F7A),
    wireValue: '#ec4899',
  ),
  AccentColor(
    name: 'Iris',
    light: Color(0xFF6C4EF0),
    dark: Color(0xFF6C4EF0),
    wireValue: '#c0a2f1',
  ),
  AccentColor(
    name: 'Cobalt',
    light: Color(0xFF2B4FE0),
    dark: Color(0xFF2B4FE0),
    wireValue: '#3b82f6',
  ),
  AccentColor(
    name: 'Jade',
    light: Color(0xFF0B8467),
    dark: Color(0xFF0B8467),
    wireValue: '#22c55e',
  ),
];

/// Resolves a v1 accent: each choice's own value, desktop's remaining legacy
/// values via their closest family, and family names written by earlier
/// preview builds. Anything else falls back to Chartreuse.
int chatColorIndex(String wireValue) {
  final normalized = switch (wireValue) {
    '#06b6d4' => '#22c55e',
    '#6366f1' => '#3b82f6',
    '#ef4444' => '#ec4899',
    '#a855f7' => '#c0a2f1',
    _ => wireValue,
  };
  final index = chatColorChoices.indexWhere(
    (color) =>
        color.wireValue == normalized || color.name.toLowerCase() == normalized,
  );
  return index < 0 ? 1 : index;
}

/// Desktop-compatible value for a family name written by earlier preview
/// builds, so those preferences sync again once republished.
String? previewFamilyWireValue(String value) {
  for (final color in chatColorChoices) {
    if (color.name.toLowerCase() == value) return color.wireValue;
  }
  return null;
}

/// Resolves a picker swatch for the active appearance mode.
Color chatColorForScheme(ColorScheme scheme, int index) {
  final color = chatColorChoices[index];
  return scheme.brightness == Brightness.dark ? color.dark : color.light;
}

/// Bubble text is white unless white falls below 4.5:1 on the fill, as on
/// Chartreuse and dark-mode Gray; those light fills take black instead.
Color chatForegroundForScheme(ColorScheme scheme, int index) {
  final fill = chatColorForScheme(scheme, index).computeLuminance();
  return 1.05 / (fill + .05) >= 4.5 ? Colors.white : Colors.black;
}

/// Translucent code surface behind [ink] on an accent bubble. It shades toward
/// the opposite of the ink, so it can only raise contrast over the fill.
Color chatCodeTint(Color ink, double opacity) =>
    (ink == Colors.white ? Colors.black : Colors.white).withValues(
      alpha: opacity,
    );

/// Received messages use their own surface rather than a control fill.
Color receivedMessageSurface(Brightness brightness) =>
    brightness == Brightness.dark
    ? const Color(0xFF292929)
    : const Color(0xFFF3F3F4);
