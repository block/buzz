import 'package:flutter/material.dart';

/// Accent colors matching the desktop Buzz app.
class AccentColor {
  final String name;
  final Color light;
  final Color dark;
  final bool useThemeForegroundInDark;
  final String wireValue;

  const AccentColor({
    required this.name,
    required this.light,
    required this.dark,
    this.useThemeForegroundInDark = false,
    required this.wireValue,
  });
}

const accentColors = [
  AccentColor(
    name: 'Neutral',
    light: Color(0xFF000000),
    dark: Color(0xFFE1E4E8),
    wireValue: 'neutral',
    useThemeForegroundInDark: true,
  ),
  AccentColor(
    name: 'Blue',
    light: Color(0xFF3B82F6),
    dark: Color(0xFF60A5FA),
    wireValue: '#3b82f6',
  ),
  AccentColor(
    name: 'Cyan',
    light: Color(0xFF06B6D4),
    dark: Color(0xFF22D3EE),
    wireValue: '#06b6d4',
  ),
  AccentColor(
    name: 'Green',
    light: Color(0xFF22C55E),
    dark: Color(0xFF4ADE80),
    wireValue: '#22c55e',
  ),
  AccentColor(
    name: 'Orange',
    light: Color(0xFFF97316),
    dark: Color(0xFFFB923C),
    wireValue: '#f97316',
  ),
  AccentColor(
    name: 'Red',
    light: Color(0xFFEF4444),
    dark: Color(0xFFF87171),
    wireValue: '#ef4444',
  ),
  AccentColor(
    name: 'Pink',
    light: Color(0xFFEC4899),
    dark: Color(0xFFF472B6),
    wireValue: '#ec4899',
  ),
  AccentColor(
    name: 'Lilac',
    light: Color(0xFFC0A2F1),
    dark: Color(0xFFC0A2F1),
    wireValue: '#c0a2f1',
  ),
  AccentColor(
    name: 'Purple',
    light: Color(0xFFA855F7),
    dark: Color(0xFFC084FC),
    wireValue: '#a855f7',
  ),
  AccentColor(
    name: 'Indigo',
    light: Color(0xFF6366F1),
    dark: Color(0xFF818CF8),
    wireValue: '#6366f1',
  ),
];

Color accentColorForScheme(ColorScheme scheme, int accentIndex) {
  if (accentIndex < 0 || accentIndex >= accentColors.length) {
    return scheme.primary;
  }
  final accent = accentColors[accentIndex];
  if (scheme.brightness == Brightness.dark && accent.useThemeForegroundInDark) {
    return scheme.onSurface;
  }
  return scheme.brightness == Brightness.light ? accent.light : accent.dark;
}

/// New default: Black.
///
/// Keep this at the end of [accentColors] so existing saved accent indexes keep
/// pointing at the same colors.
const neutralAccentIndex = 0;
const defaultAccentIndex = neutralAccentIndex;

/// Legacy default: Catppuccin Mauve/the base theme primary.
const legacyDefaultAccentIndex = -1;

int? accentIndexForWireValue(String value) {
  final index = accentColors.indexWhere((accent) => accent.wireValue == value);
  return index < 0 ? null : index;
}

String legacyAccentWireValue(int? index) {
  // Legacy mobile indexes 0...7 match desktop Blue...Indigo except Lilac,
  // which did not exist. Black (8) has no desktop wire value, so migrate it
  // deterministically to Neutral rather than publishing a mobile-only value.
  if (index == 8) return 'neutral';
  if (index != null && index >= 0 && index <= 5) {
    return accentColors[index + 1].wireValue;
  }
  if (index == 6) return '#a855f7';
  if (index == 7) return '#6366f1';
  return '#3b82f6';
}
