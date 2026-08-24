import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'accent_colors.dart';
import 'theme_catalog.dart';
import 'theme_provider.dart' show effectiveTheme, schemeForAppearanceMode;

const communityThemeDTag = 'community-theme';
const defaultCommunityGlassBackground = false;
const defaultCommunityGlassOpacity = 65;
const minCommunityGlassOpacity = 30;
const maxCommunityGlassOpacity = 90;
const defaultCommunityProminentActiveTab = false;
const defaultCommunityTheme = CommunityThemePreference(
  theme: 'buzz',
  accent: '#3b82f6',
  followSystem: true,
  // A fresh community carries no desktop appearance opinion, so it must not
  // serialize the desktop-only fields and overwrite a real desktop record.
  includesGlassBackground: false,
  includesGlassOpacity: false,
  includesProminentActiveTab: false,
);

class CommunityThemePreference {
  final int version;
  final String theme;
  final String accent;
  final bool followSystem;
  // These appearance values are synced even though mobile cannot render
  // desktop glass. Keeping them in the model makes mobile a lossless client.
  final bool glassBackground;
  final int glassOpacity;
  final bool prominentActiveTab;
  final bool includesGlassBackground;
  final bool includesGlassOpacity;
  final bool includesProminentActiveTab;

  const CommunityThemePreference({
    this.version = 1,
    required this.theme,
    required this.accent,
    required this.followSystem,
    this.glassBackground = defaultCommunityGlassBackground,
    this.glassOpacity = defaultCommunityGlassOpacity,
    this.prominentActiveTab = defaultCommunityProminentActiveTab,
    this.includesGlassBackground = true,
    this.includesGlassOpacity = true,
    this.includesProminentActiveTab = true,
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
    final includesGlassBackground = json.containsKey('glassBackground');
    final includesGlassOpacity = json.containsKey('glassOpacity');
    final includesProminentActiveTab = json.containsKey('prominentActiveTab');
    final glassBackground = includesGlassBackground
        ? json['glassBackground']
        : defaultCommunityGlassBackground;
    final glassOpacity = includesGlassOpacity
        ? json['glassOpacity']
        : defaultCommunityGlassOpacity;
    final prominentActiveTab = includesProminentActiveTab
        ? json['prominentActiveTab']
        : defaultCommunityProminentActiveTab;
    if (glassBackground is! bool ||
        glassOpacity is! num ||
        !glassOpacity.isFinite ||
        glassOpacity != glassOpacity.round() ||
        glassOpacity < minCommunityGlassOpacity ||
        glassOpacity > maxCommunityGlassOpacity ||
        prominentActiveTab is! bool) {
      throw const FormatException('Invalid community theme preference');
    }
    return CommunityThemePreference(
      theme: json['theme'] as String,
      accent: json['accent'] as String,
      followSystem: json['followSystem'] as bool,
      glassBackground: glassBackground,
      glassOpacity: glassOpacity.toInt(),
      prominentActiveTab: prominentActiveTab,
      includesGlassBackground: includesGlassBackground,
      includesGlassOpacity: includesGlassOpacity,
      includesProminentActiveTab: includesProminentActiveTab,
    );
  }

  Map<String, dynamic> toJson() => {
    'version': version,
    'theme': theme,
    'accent': accent,
    'followSystem': followSystem,
    if (includesGlassBackground) 'glassBackground': glassBackground,
    if (includesGlassOpacity) 'glassOpacity': glassOpacity,
    if (includesProminentActiveTab) 'prominentActiveTab': prominentActiveTab,
  };

  CommunityThemePreference copyWith({
    String? theme,
    String? accent,
    bool? followSystem,
    bool? glassBackground,
    int? glassOpacity,
    bool? prominentActiveTab,
  }) => CommunityThemePreference(
    version: version,
    theme: theme ?? this.theme,
    accent: accent ?? this.accent,
    followSystem: followSystem ?? this.followSystem,
    glassBackground: glassBackground ?? this.glassBackground,
    glassOpacity: glassOpacity ?? this.glassOpacity,
    prominentActiveTab: prominentActiveTab ?? this.prominentActiveTab,
    includesGlassBackground: glassBackground != null || includesGlassBackground,
    includesGlassOpacity: glassOpacity != null || includesGlassOpacity,
    includesProminentActiveTab:
        prominentActiveTab != null || includesProminentActiveTab,
  );

  ThemeMode get mode {
    if (followSystem) return ThemeMode.system;
    return findTheme(theme)?.isDark == true ? ThemeMode.dark : ThemeMode.light;
  }

  /// Whether this preference carries a full desktop appearance opinion. A
  /// preference parsed from a legacy three-field payload, or a fresh/legacy
  /// mobile origin, omits these fields and must not replace a desktop record.
  bool get includesDesktopAppearance =>
      includesGlassBackground &&
      includesGlassOpacity &&
      includesProminentActiveTab;

  /// Adopt [source]'s desktop-only appearance in place of this preference's.
  /// Mobile has no UI that authors glass or prominent-tab, so those fields are
  /// never a local opinion — they are only ever a copy of what was hydrated.
  /// A cached "full" preference therefore carries stale desktop fields once
  /// another client changes them, so republishing a mobile edit must take the
  /// desktop-only fields from the latest observed coordinate rather than trust
  /// its own cache; otherwise it silently replays stale glass over the relay.
  CommunityThemePreference mergeDesktopAppearanceFrom(
    CommunityThemePreference source,
  ) {
    return CommunityThemePreference(
      version: version,
      theme: theme,
      accent: accent,
      followSystem: followSystem,
      glassBackground: source.glassBackground,
      glassOpacity: source.glassOpacity,
      prominentActiveTab: source.prominentActiveTab,
      includesGlassBackground: source.includesGlassBackground,
      includesGlassOpacity: source.includesGlassOpacity,
      includesProminentActiveTab: source.includesProminentActiveTab,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is CommunityThemePreference &&
      theme == other.theme &&
      accent == other.accent &&
      followSystem == other.followSystem &&
      glassBackground == other.glassBackground &&
      glassOpacity == other.glassOpacity &&
      prominentActiveTab == other.prominentActiveTab &&
      includesGlassBackground == other.includesGlassBackground &&
      includesGlassOpacity == other.includesGlassOpacity &&
      includesProminentActiveTab == other.includesProminentActiveTab;

  @override
  int get hashCode => Object.hash(
    theme,
    accent,
    followSystem,
    glassBackground,
    glassOpacity,
    prominentActiveTab,
    includesGlassBackground,
    includesGlassOpacity,
    includesProminentActiveTab,
  );
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
      // The pre-per-community mobile theme has no desktop appearance opinion,
      // so it must not publish desktop-only defaults over a real desktop record.
      includesGlassBackground: false,
      includesGlassOpacity: false,
      includesProminentActiveTab: false,
    );
  }
}

String normalizeCommunityRelayUrl(String relayUrl) =>
    relayUrl.trim().replaceFirst(RegExp(r'/+$'), '').toLowerCase();
