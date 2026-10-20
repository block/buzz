import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('desktop surface and text roles match in both modes', () {
    final light = mobileAppearanceTheme(Brightness.light, 'neutral');
    final dark = mobileAppearanceTheme(Brightness.dark, 'neutral');
    expect(light.colorScheme.surface, const Color(0xFFFFFFFF));
    expect(light.colorScheme.onSurface, const Color(0xFF0A0A0A));
    expect(light.colorScheme.onSurfaceVariant, const Color(0xFF646464));
    expect(light.colorScheme.outline, const Color(0xFFE8E8E8));
    expect(dark.colorScheme.surface, const Color(0xFF000000));
    expect(dark.colorScheme.onSurface, const Color(0xFFFFFFFF));
    expect(dark.colorScheme.onSurfaceVariant, const Color(0xFFC7C7C7));
    expect(dark.colorScheme.outline, const Color(0xFF1A1A1A));
  });

  for (final brightness in Brightness.values) {
    test('chat colors do not recolor controls in $brightness', () {
      final base = mobileAppearanceTheme(brightness, 'neutral');
      for (final accent in accentColors) {
        final theme = mobileAppearanceTheme(brightness, accent.wireValue);
        expect(theme.colorScheme, base.colorScheme);
        final fill = theme.extension<ChatColors>()!.outgoing;
        final foreground = theme.extension<ChatColors>()!.foreground;
        final a = fill.computeLuminance();
        final b = foreground.computeLuminance();
        final ratio = a > b ? (a + .05) / (b + .05) : (b + .05) / (a + .05);
        expect(ratio, greaterThanOrEqualTo(4.5));
      }
    });
  }
}
