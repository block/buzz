import 'package:flutter/material.dart';

import 'accent_colors.dart';

/// Desktop accent families in picker order. Fills are accent-9 from buzz-app's
/// design-system tokens (4ad00a8a).
const chatColorChoices = [
  AccentColor(
    name: 'Gray',
    light: Color(0xFF525252),
    dark: Color(0xFFC7C7C7),
    wireValue: 'gray',
  ),
  AccentColor(
    name: 'Chartreuse',
    light: Color(0xFFD7D72E),
    dark: Color(0xFFD7D72E),
    wireValue: 'chartreuse',
  ),
  AccentColor(
    name: 'Flame',
    light: Color(0xFFD93A0C),
    dark: Color(0xFFD93A0C),
    wireValue: 'flame',
  ),
  AccentColor(
    name: 'Magenta',
    light: Color(0xFFCC1F7A),
    dark: Color(0xFFCC1F7A),
    wireValue: 'magenta',
  ),
  AccentColor(
    name: 'Iris',
    light: Color(0xFF6C4EF0),
    dark: Color(0xFF6C4EF0),
    wireValue: 'iris',
  ),
  AccentColor(
    name: 'Cobalt',
    light: Color(0xFF2B4FE0),
    dark: Color(0xFF2B4FE0),
    wireValue: 'cobalt',
  ),
  AccentColor(
    name: 'Jade',
    light: Color(0xFF0B8467),
    dark: Color(0xFF0B8467),
    wireValue: 'jade',
  ),
];

/// Carries saved mobile colors forward using desktop's legacy-family mapping.
/// The retired Neutral choice becomes the brand default, Chartreuse.
int chatColorIndex(String wireValue) {
  final normalized = switch (wireValue) {
    '#3b82f6' || '#06b6d4' || '#6366f1' => 'cobalt',
    '#22c55e' => 'jade',
    '#f97316' => 'flame',
    '#ec4899' || '#ef4444' => 'magenta',
    '#c0a2f1' || '#a855f7' => 'iris',
    'neutral' => 'chartreuse',
    _ => wireValue,
  };
  final index = chatColorChoices.indexWhere(
    (color) => color.wireValue == normalized,
  );
  return index < 0 ? 1 : index;
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

/// Received messages use their own surface rather than a control fill.
Color receivedMessageSurface(Brightness brightness) =>
    brightness == Brightness.dark
    ? const Color(0xFF292929)
    : const Color(0xFFF3F3F4);
