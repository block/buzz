import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test('complete desktop v1 payload round-trips exactly', () {
    final preference = CommunityThemePreference.fromJson({
      'version': 1,
      'theme': 'github-dark',
      'accent': '#c0a2f1',
      'followSystem': false,
      'glassBackground': true,
      'glassOpacity': 47,
      'prominentActiveTab': false,
    });

    expect(preference.mode, ThemeMode.dark);
    expect(
      jsonEncode(preference.toJson()),
      '{"version":1,"theme":"github-dark","accent":"#c0a2f1","followSystem":false,"glassBackground":true,"glassOpacity":47,"prominentActiveTab":false}',
    );
  });

  test('older v1 payloads retain missing desktop-only fields', () {
    final preference = CommunityThemePreference.fromJson({
      'version': 1,
      'theme': 'buzz',
      'accent': '#3b82f6',
      'followSystem': true,
    });

    expect(preference.glassBackground, isFalse);
    expect(preference.glassOpacity, 65);
    expect(preference.prominentActiveTab, isFalse);
    expect(preference.toJson(), isNot(contains('glassBackground')));
    expect(preference.toJson(), isNot(contains('glassOpacity')));
    expect(preference.toJson(), isNot(contains('prominentActiveTab')));
  });

  test('mobile edits do not widen legacy appearance fields with defaults', () {
    final legacy = CommunityThemePreference.fromJson({
      'version': 1,
      'theme': 'buzz',
      'accent': '#3b82f6',
      'followSystem': true,
    });

    final edited = legacy.copyWith(accent: '#ef4444');

    expect(edited.toJson(), {
      'version': 1,
      'theme': 'buzz',
      'accent': '#ef4444',
      'followSystem': true,
    });

    expect(legacy.copyWith(glassOpacity: 70).toJson(), {
      'version': 1,
      'theme': 'buzz',
      'accent': '#3b82f6',
      'followSystem': true,
      'glassOpacity': 70,
    });
  });

  test('mobile appearance limits match the shared wire contract', () {
    final contract =
        jsonDecode(File('../schema/community-theme-v1.json').readAsStringSync())
            as Map<String, dynamic>;
    final properties = contract['properties'] as Map<String, dynamic>;
    final glassBackground =
        properties['glassBackground'] as Map<String, dynamic>;
    final glassOpacity = properties['glassOpacity'] as Map<String, dynamic>;
    final prominentActiveTab =
        properties['prominentActiveTab'] as Map<String, dynamic>;
    final theme = properties['theme'] as Map<String, dynamic>;
    final accent = properties['accent'] as Map<String, dynamic>;

    expect(defaultCommunityGlassBackground, glassBackground['default'] as bool);
    expect(defaultCommunityGlassOpacity, glassOpacity['default'] as int);
    expect(minCommunityGlassOpacity, glassOpacity['minimum'] as int);
    expect(maxCommunityGlassOpacity, glassOpacity['maximum'] as int);
    expect(
      defaultCommunityProminentActiveTab,
      prominentActiveTab['default'] as bool,
    );
    expect(
      theme['enum'],
      unorderedEquals(themeCatalog.map((entry) => entry.name)),
    );
    expect(
      accent['enum'],
      unorderedEquals(accentColors.map((entry) => entry.wireValue)),
    );
  });

  test('rejects invalid theme and appearance values', () {
    for (final payload in [
      {
        'version': 2,
        'theme': 'buzz',
        'accent': '#3b82f6',
        'followSystem': true,
      },
      {
        'version': 1,
        'theme': 'unknown',
        'accent': '#3b82f6',
        'followSystem': true,
      },
      {
        'version': 1,
        'theme': 'buzz',
        'accent': '#000000',
        'followSystem': true,
      },
      {
        'version': 1,
        'theme': 'buzz',
        'accent': '#3b82f6',
        'followSystem': true,
        'glassBackground': 'true',
      },
      {
        'version': 1,
        'theme': 'buzz',
        'accent': '#3b82f6',
        'followSystem': true,
        'glassOpacity': 29,
      },
      {
        'version': 1,
        'theme': 'buzz',
        'accent': '#3b82f6',
        'followSystem': true,
        'prominentActiveTab': 1,
      },
      {
        'version': 1,
        'theme': 'buzz',
        'accent': '#3b82f6',
        'followSystem': true,
        'glassBackground': null,
      },
      {
        'version': 1,
        'theme': 'buzz',
        'accent': '#3b82f6',
        'followSystem': true,
        'glassOpacity': null,
      },
      {
        'version': 1,
        'theme': 'buzz',
        'accent': '#3b82f6',
        'followSystem': true,
        'prominentActiveTab': null,
      },
    ]) {
      expect(
        () => CommunityThemePreference.fromJson(payload),
        throwsFormatException,
      );
    }
  });

  test('a fresh community default omits desktop-only appearance fields', () {
    // Regression: a pre-hydration mobile edit built from the default must not
    // publish false/65/false over a real desktop record on the relay.
    expect(defaultCommunityTheme.toJson(), {
      'version': 1,
      'theme': 'buzz',
      'accent': '#3b82f6',
      'followSystem': true,
    });
    expect(defaultCommunityTheme.copyWith(accent: '#ef4444').toJson(), {
      'version': 1,
      'theme': 'buzz',
      'accent': '#ef4444',
      'followSystem': true,
    });
  });

  test('supported edits preserve appearance values mobile cannot render', () {
    const desktopPreference = CommunityThemePreference(
      theme: 'buzz',
      accent: '#3b82f6',
      followSystem: true,
      glassBackground: true,
      glassOpacity: 42,
      prominentActiveTab: false,
    );

    final mobileEdit = desktopPreference.copyWith(theme: 'dracula');

    expect(mobileEdit.theme, 'dracula');
    expect(mobileEdit.glassBackground, isTrue);
    expect(mobileEdit.glassOpacity, 42);
    expect(mobileEdit.prominentActiveTab, isFalse);
  });

  test('storage is scoped by pubkey and normalized relay URL', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final storage = CommunityThemeStorage(prefs);
    const a = CommunityThemePreference(
      theme: 'buzz',
      accent: '#3b82f6',
      followSystem: true,
    );
    const b = CommunityThemePreference(
      theme: 'dracula',
      accent: '#ef4444',
      followSystem: false,
    );

    await storage.write('pk', 'WSS://Relay.Example///', a);
    await storage.write('pk', 'wss://other.example', b);

    expect(storage.read('pk', 'wss://relay.example'), a);
    expect(storage.read('pk', 'wss://other.example/'), b);
    expect(storage.read('other-pk', 'wss://relay.example'), isNull);
  });

  test('dirty outbox survives restart and clears only exact ack', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final storage = CommunityThemeStorage(prefs);
    const pending = CommunityThemePreference(
      theme: 'dracula',
      accent: '#ef4444',
      followSystem: false,
    );
    const newer = CommunityThemePreference(
      theme: 'houston',
      accent: '#a855f7',
      followSystem: false,
    );

    await storage.writeOutbox('pk', 'wss://relay.example', pending);
    expect(
      CommunityThemeStorage(prefs).readOutbox('pk', 'wss://relay.example'),
      pending,
    );
    await storage.writeOutbox('pk', 'wss://relay.example', newer);
    await storage.clearOutbox('pk', 'wss://relay.example', pending);
    expect(storage.readOutbox('pk', 'wss://relay.example'), newer);
    await storage.clearOutbox('pk', 'wss://relay.example', newer);
    expect(storage.readOutbox('pk', 'wss://relay.example'), isNull);
  });

  test('legacy accent indexes migrate without inventing wire values', () async {
    SharedPreferences.setMockInitialValues({
      'buzz_theme_mode': 'dark',
      'buzz_color_scheme': 'dracula',
      'buzz_accent_color': 8,
    });
    final prefs = await SharedPreferences.getInstance();
    final preference = CommunityThemeStorage(prefs).legacyPreference();

    expect(
      preference,
      const CommunityThemePreference(
        theme: 'dracula',
        accent: 'neutral',
        followSystem: false,
        // The legacy mobile theme carries no desktop appearance opinion, so it
        // must not serialize desktop-only defaults over a real desktop record.
        includesGlassBackground: false,
        includesGlassOpacity: false,
        includesProminentActiveTab: false,
      ),
    );
    expect(preference.toJson(), isNot(contains('glassBackground')));
    expect(preference.toJson(), isNot(contains('glassOpacity')));
    expect(preference.toJson(), isNot(contains('prominentActiveTab')));
    expect(preference.mode, ThemeMode.dark);
    expect(legacyAccentWireValue(6), '#a855f7');
    expect(legacyAccentWireValue(7), '#6366f1');
  });
}
