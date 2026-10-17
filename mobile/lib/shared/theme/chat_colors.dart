import 'package:flutter/material.dart';

import 'accent_colors.dart';

/// Desktop message palette, in picker order. Wire values remain compatible
/// with existing community preferences; fills match buzz-app bubble tokens.
const chatColorChoices = [
  AccentColor(
    name: 'Neutral',
    light: Color(0xFFE9E9EA),
    dark: Color(0xFF2E2E2E),
    wireValue: 'neutral',
  ),
  AccentColor(
    name: 'Blue',
    light: Color(0xFF0077DB),
    dark: Color(0xFF0077DB),
    wireValue: '#3b82f6',
  ),
  AccentColor(
    name: 'Green',
    light: Color(0xFF008934),
    dark: Color(0xFF008934),
    wireValue: '#22c55e',
  ),
  AccentColor(
    name: 'Orange',
    light: Color(0xFFC95100),
    dark: Color(0xFFC95100),
    wireValue: '#f97316',
  ),
  AccentColor(
    name: 'Pink',
    light: Color(0xFFD33785),
    dark: Color(0xFFD33785),
    wireValue: '#ec4899',
  ),
  AccentColor(
    name: 'Lilac',
    light: Color(0xFFA94AD8),
    dark: Color(0xFFA94AD8),
    wireValue: '#c0a2f1',
  ),
];

/// Resolves old community colors using desktop's retired-color aliases.
int chatColorIndex(String wireValue) {
  final normalized = switch (wireValue) {
    '#06b6d4' || '#6366f1' => '#3b82f6',
    '#ef4444' => '#ec4899',
    '#a855f7' => '#c0a2f1',
    _ => wireValue,
  };
  final index = chatColorChoices.indexWhere(
    (color) => color.wireValue == normalized,
  );
  return index < 0 ? 0 : index;
}

/// Resolves a picker swatch for the active appearance mode.
Color chatColorForScheme(ColorScheme scheme, int index) {
  final color = chatColorChoices[index];
  return scheme.brightness == Brightness.dark ? color.dark : color.light;
}
