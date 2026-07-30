import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'accent_colors.dart';
import 'theme_catalog.dart';
import 'theme_provider.dart' show effectiveTheme, schemeForAppearanceMode;

const communityThemeDTag = 'community-theme';
const defaultCommunityTheme = CommunityThemePreference(
  theme: 'buzz',
  accent: '#3b82f6',
  followSystem: true,
);

class CommunityThemePreference {
  final int version;
  final String theme;
  final String accent;
  final bool followSystem;

  const CommunityThemePreference({
    this.version = 1,
    required this.theme,
    required this.accent,
    required this.followSystem,
  });

  factory CommunityThemePreference.fromJson(Map<String, dynamic> json) {
    if (json['version'] != 1 ||
        json['theme'] is! String ||
        findTheme(json['theme'] as String) == null ||
        json['accent'] is! String ||
        accentIndexForWireValue(json['accent'] as String) == null ||
        json['followSystem'] is! bool) {
      throw const FormatException('Invalid community theme preference');
    }
    return CommunityThemePreference(
      theme: json['theme'] as String,
      accent: json['accent'] as String,
      followSystem: json['followSystem'] as bool,
    );
  }

  Map<String, dynamic> toJson() => {
    'version': version,
    'theme': theme,
    'accent': accent,
    'followSystem': followSystem,
  };

  ThemeMode get mode {
    if (followSystem) return ThemeMode.system;
    return findTheme(theme)?.isDark == true ? ThemeMode.dark : ThemeMode.light;
  }

  @override
  bool operator ==(Object other) =>
      other is CommunityThemePreference &&
      theme == other.theme &&
      accent == other.accent &&
      followSystem == other.followSystem;

  @override
  int get hashCode => Object.hash(theme, accent, followSystem);
}

class CommunityThemeStorage {
  static const _prefix = 'buzz-community-theme.v1';
  static const _outboxPrefix = 'buzz-community-theme-outbox.v1';
  static const _migrationPrefix = 'buzz-community-theme-migrated.v1';
  static const _legacyModeKey = 'buzz_theme_mode';
  static const _legacyAccentKey = 'buzz_accent_color';
  static const _legacySchemeKey = 'buzz_color_scheme';

  final SharedPreferences prefs;

  const CommunityThemeStorage(this.prefs);

  String key(String pubkey, String relayUrl) =>
      '$_prefix:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

  String outboxKey(String pubkey, String relayUrl) =>
      '$_outboxPrefix:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

  CommunityThemePreference? _readKey(String storageKey) {
    try {
      final raw = prefs.getString(storageKey);
      if (raw == null) return null;
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, dynamic>) return null;
      return CommunityThemePreference.fromJson(decoded);
    } catch (_) {
      return null;
    }
  }

  CommunityThemePreference? read(String pubkey, String relayUrl) =>
      _readKey(key(pubkey, relayUrl));

  CommunityThemePreference? readOutbox(String pubkey, String relayUrl) =>
      _readKey(outboxKey(pubkey, relayUrl));

  Future<bool> write(
    String pubkey,
    String relayUrl,
    CommunityThemePreference preference,
  ) => prefs.setString(key(pubkey, relayUrl), jsonEncode(preference.toJson()));

  Future<bool> writeOutbox(
    String pubkey,
    String relayUrl,
    CommunityThemePreference preference,
  ) => prefs.setString(
    outboxKey(pubkey, relayUrl),
    jsonEncode(preference.toJson()),
  );

  Future<void> clearOutbox(
    String pubkey,
    String relayUrl,
    CommunityThemePreference acknowledged,
  ) async {
    if (readOutbox(pubkey, relayUrl) == acknowledged) {
      await prefs.remove(outboxKey(pubkey, relayUrl));
    }
  }

  bool hasMigrated(String pubkey) =>
      prefs.getBool('$_migrationPrefix:$pubkey') == true;

  Future<bool> markMigrated(String pubkey) =>
      prefs.setBool('$_migrationPrefix:$pubkey', true);

  Future<void> writeLegacy(CommunityThemePreference preference) async {
    await prefs.setString(_legacyModeKey, preference.mode.name);
    await prefs.setString(_legacySchemeKey, preference.theme);
    await prefs.setInt(
      _legacyAccentKey,
      accentIndexForWireValue(preference.accent) ?? defaultAccentIndex,
    );
  }

  CommunityThemePreference legacyPreference() {
    final modeName = prefs.getString(_legacyModeKey);
    final mode =
        ThemeMode.values.where((value) => value.name == modeName).firstOrNull ??
        ThemeMode.system;
    final storedTheme = prefs.getString(_legacySchemeKey);
    final theme = findTheme(storedTheme ?? 'buzz')?.name ?? 'buzz';
    final legacyAccent = prefs.getInt(_legacyAccentKey);
    final resolvedTheme = switch (mode) {
      ThemeMode.system => schemeForAppearanceMode(theme, mode) ?? theme,
      ThemeMode.light ||
      ThemeMode.dark => effectiveTheme(theme, mode)?.name ?? theme,
    };
    return CommunityThemePreference(
      theme: resolvedTheme,
      accent: legacyAccentWireValue(legacyAccent),
      followSystem: mode == ThemeMode.system,
    );
  }
}

String normalizeCommunityRelayUrl(String relayUrl) =>
    relayUrl.trim().replaceFirst(RegExp(r'/+$'), '').toLowerCase();
