import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('matches all seven desktop accent fills and readable inks', () {
    expect(chatColorChoices.map((color) => color.name), [
      'Gray',
      'Chartreuse',
      'Flame',
      'Magenta',
      'Iris',
      'Cobalt',
      'Jade',
    ]);
    expect(chatColorChoices.map((color) => color.light.toARGB32()), [
      0xFF525252,
      0xFFD7D72E,
      0xFFD93A0C,
      0xFFCC1F7A,
      0xFF6C4EF0,
      0xFF2B4FE0,
      0xFF0B8467,
    ]);
    expect(chatColorChoices.map((color) => color.dark.toARGB32()), [
      0xFFC7C7C7,
      0xFFD7D72E,
      0xFFD93A0C,
      0xFFCC1F7A,
      0xFF6C4EF0,
      0xFF2B4FE0,
      0xFF0B8467,
    ]);
    for (final brightness in Brightness.values) {
      final scheme = brightness == Brightness.dark
          ? darkColorScheme
          : lightColorScheme;
      for (var index = 0; index < chatColorChoices.length; index++) {
        final ink = chatForegroundForScheme(scheme, index);
        final a = ink.computeLuminance();
        final b = chatColorForScheme(scheme, index).computeLuminance();
        final contrast = a > b ? (a + .05) / (b + .05) : (b + .05) / (a + .05);
        expect(
          contrast,
          greaterThanOrEqualTo(4.5),
          reason: '${chatColorChoices[index].name} $brightness',
        );
        // Only the light fills fall back to black: Chartreuse and dark Gray.
        final black = index == 1 || (index == 0 && scheme == darkColorScheme);
        expect(ink, black ? Colors.black : Colors.white);
      }
    }
  });

  test(
    'retired saved colors follow desktop aliases without losing preferences',
    () {
      for (final entry in {
        '#3b82f6': 'Cobalt',
        '#06b6d4': 'Cobalt',
        '#6366f1': 'Cobalt',
        '#22c55e': 'Jade',
        '#f97316': 'Flame',
        '#ec4899': 'Magenta',
        '#c0a2f1': 'Iris',
        'neutral': 'Chartreuse',
        '#ef4444': 'Magenta',
        '#a855f7': 'Iris',
      }.entries) {
        expect(chatColorChoices[chatColorIndex(entry.key)].name, entry.value);
        final preference = CommunityThemePreference.fromJson({
          'version': 1,
          'theme': 'buzz-dark',
          'accent': entry.key,
          'followSystem': false,
        });
        expect(preference.mode, ThemeMode.dark);
        expect(preference.accent, entry.key);
        expect(
          mobileAppearanceTheme(
            Brightness.dark,
            preference.accent,
          ).extension<ChatColors>()!.outgoing,
          chatColorChoices[chatColorIndex(entry.key)].dark,
        );
      }
    },
  );
}
