import 'package:flutter/material.dart';

/// Accent colors matching the desktop Sprout app.
class AccentColor {
  final String name;
  final Color light;
  final Color dark;

  const AccentColor({
    required this.name,
    required this.light,
    required this.dark,
  });
}

const accentColors = [
  AccentColor(name: 'Blue', light: Color(0xFF3B82F6), dark: Color(0xFF60A5FA)),
  AccentColor(name: 'Cyan', light: Color(0xFF06B6D4), dark: Color(0xFF22D3EE)),
  AccentColor(name: 'Green', light: Color(0xFF22C55E), dark: Color(0xFF4ADE80)),
  AccentColor(
    name: 'Orange',
    light: Color(0xFFF97316),
    dark: Color(0xFFFB923C),
  ),
  AccentColor(name: 'Red', light: Color(0xFFEF4444), dark: Color(0xFFF87171)),
  AccentColor(name: 'Pink', light: Color(0xFFEC4899), dark: Color(0xFFF472B6)),
  AccentColor(
    name: 'Purple',
    light: Color(0xFFA855F7),
    dark: Color(0xFFC084FC),
  ),
  AccentColor(
    name: 'Indigo',
    light: Color(0xFF6366F1),
    dark: Color(0xFF818CF8),
  ),
];

/// Default: Catppuccin Mauve (the current primary).
const defaultAccentIndex = -1; // -1 means "use theme default (Mauve)"
