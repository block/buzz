import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('matches the six desktop choices and bubble fills in order', () {
    expect(chatColorChoices.map((color) => color.name), [
      'Neutral',
      'Blue',
      'Green',
      'Orange',
      'Pink',
      'Lilac',
    ]);
    expect(chatColorChoices.map((color) => color.light.toARGB32()), [
      0xFFE9E9EA,
      0xFF0077DB,
      0xFF008934,
      0xFFC95100,
      0xFFD33785,
      0xFFA94AD8,
    ]);
    expect(chatColorChoices.map((color) => color.dark.toARGB32()), [
      0xFF2E2E2E,
      0xFF0077DB,
      0xFF008934,
      0xFFC95100,
      0xFFD33785,
      0xFFA94AD8,
    ]);
  });

  test(
    'retired saved colors follow desktop aliases without losing preferences',
    () {
      for (final entry in {
        '#06b6d4': 'Blue',
        '#6366f1': 'Blue',
        '#ef4444': 'Pink',
        '#a855f7': 'Lilac',
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
