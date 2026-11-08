import 'dart:convert';
import 'dart:io';

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

  test('accents follow the cross-client v1 contract', () {
    // Desktop's parser test reads the same fixture, so both clients agree on
    // every value either can save.
    final contract =
        jsonDecode(
              File(
                '../test-fixtures/community-theme-accents.json',
              ).readAsStringSync(),
            )
            as Map<String, dynamic>;
    CommunityThemePreference parse(String accent) =>
        CommunityThemePreference.fromJson({
          'version': 1,
          'theme': 'buzz-dark',
          'accent': accent,
          'followSystem': false,
        });

    final emitted = (contract['emitted'] as List).cast<Map<String, dynamic>>();
    expect(
      emitted.map((entry) => entry['family']),
      chatColorChoices.map((color) => color.name),
    );
    for (final (index, entry) in emitted.indexed) {
      final accent = entry['accent'] as String;
      expect(chatColorChoices[index].wireValue, accent);
      // Family names from earlier preview builds republish as the v1 value.
      expect(parse(chatColorChoices[index].name.toLowerCase()).accent, accent);
    }
    // Older clients reject an unknown accent with the whole preference, so
    // anything published without a picker action stays in their set.
    final baseReaders = ((contract['baseReaders'] as Map)['accepted'] as List)
        .cast<String>();
    for (final automatic in [
      defaultCommunityTheme.accent,
      for (final index in [null, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8])
        legacyAccentWireValue(index),
    ]) {
      expect(baseReaders, contains(automatic));
    }

    final legacy = (contract['legacy'] as List).cast<Map<String, dynamic>>();
    for (final entry in [...emitted, ...legacy]) {
      final accent = entry['accent'] as String;
      final preference = parse(accent);
      expect(preference.mode, ThemeMode.dark);
      expect(preference.accent, accent);
      final family = chatColorChoices.firstWhere(
        (color) => color.name == entry['family'],
      );
      expect(
        mobileAppearanceTheme(
          Brightness.dark,
          preference.accent,
        ).extension<ChatColors>()!.outgoing,
        family.dark,
        reason: accent,
      );
    }

    for (final accent in (contract['rejected'] as List).cast<String>()) {
      expect(() => parse(accent), throwsFormatException, reason: accent);
    }
  });
}
