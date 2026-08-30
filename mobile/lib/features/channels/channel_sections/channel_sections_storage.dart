import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

/// Normalizes a relay URL for storage keys so equivalent URLs share a scope.
/// Matches desktop `normalizeRelayUrl` and mobile channel-sort.
String normalizeChannelSectionsRelayUrl(String relayUrl) =>
    relayUrl.trim().replaceFirst(RegExp(r'/+$'), '').toLowerCase();

/// Relay-scoped local storage key for channel sections.
String channelSectionsKey(String pubkey, String relayUrl) =>
    'buzz.channel-sections.v1:$pubkey:${Uri.encodeComponent(normalizeChannelSectionsRelayUrl(relayUrl))}';

/// Pre-scoping key (account-global). Used only for one-time migration.
String legacyChannelSectionsKey(String pubkey) =>
    'buzz.channel-sections.v1:$pubkey';

class ChannelSection {
  final String id;
  final String name;
  final String? icon;
  final int order;

  const ChannelSection({
    required this.id,
    required this.name,
    this.icon,
    required this.order,
  });

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    if (icon != null) 'icon': icon,
    'order': order,
  };

  factory ChannelSection.fromJson(Map<String, dynamic> json) => ChannelSection(
    id: json['id'] as String,
    name: json['name'] as String,
    icon: json['icon'] is String ? json['icon'] as String : null,
    order: json['order'] as int,
  );
}

class ChannelSectionStore {
  final int version;
  final List<ChannelSection> sections;
  final Map<String, String> assignments;

  const ChannelSectionStore({
    this.version = 1,
    this.sections = const [],
    this.assignments = const {},
  });

  Map<String, dynamic> toJson() => {
    'version': version,
    'sections': sections.map((s) => s.toJson()).toList(),
    'assignments': assignments,
  };

  factory ChannelSectionStore.fromJson(Map<String, dynamic> json) {
    final rawSections = json['sections'];
    final sections = <ChannelSection>[];
    if (rawSections is List) {
      for (final entry in rawSections) {
        if (entry is Map<String, dynamic> &&
            entry['id'] is String &&
            entry['name'] is String &&
            entry['order'] is int) {
          sections.add(ChannelSection.fromJson(entry));
        }
      }
    }

    final rawAssignments = json['assignments'];
    final assignments = <String, String>{};
    if (rawAssignments is Map) {
      for (final entry in rawAssignments.entries) {
        if (entry.key is String && entry.value is String) {
          assignments[entry.key as String] = entry.value as String;
        }
      }
    }

    // Strip assignments referencing sections that don't exist.
    final sectionIds = {for (final s in sections) s.id};
    assignments.removeWhere((_, sectionId) => !sectionIds.contains(sectionId));

    return ChannelSectionStore(
      version: 1,
      sections: sections,
      assignments: assignments,
    );
  }
}

class ChannelSectionsStorage {
  final SharedPreferences _prefs;

  ChannelSectionsStorage(this._prefs);

  /// Read sections for [pubkey] scoped to [relayUrl].
  ///
  /// On first access for a scoped key, migrates any existing data from the
  /// legacy pubkey-only key so users don't lose their sections on upgrade.
  /// After a successful migration the legacy key is deleted — subsequent
  /// empty scoped-key reads (a different relay) won't see legacy data.
  ChannelSectionStore read(String pubkey, String relayUrl) {
    final scoped = _readKey(channelSectionsKey(pubkey, relayUrl));
    if (scoped != null) return scoped;

    // One-time migration from account-global storage (pre-relay-scope).
    // The first active relay claims the legacy value; removing it prevents
    // the same unscoped sections from bleeding into later communities.
    final legacy = _readKey(legacyChannelSectionsKey(pubkey));
    if (legacy != null && legacy.sections.isNotEmpty) {
      write(pubkey, relayUrl, legacy);
      _prefs.remove(legacyChannelSectionsKey(pubkey));
      return legacy;
    }

    return const ChannelSectionStore();
  }

  ChannelSectionStore? _readKey(String key) {
    final raw = _prefs.getString(key);
    if (raw == null || raw.isEmpty) {
      return null;
    }

    try {
      final parsed = jsonDecode(raw);
      if (parsed is! Map<String, dynamic>) {
        return null;
      }
      if (parsed['version'] != 1) {
        return null;
      }
      return ChannelSectionStore.fromJson(parsed);
    } catch (_) {
      return null;
    }
  }

  void write(String pubkey, String relayUrl, ChannelSectionStore store) {
    _prefs.setString(
      channelSectionsKey(pubkey, relayUrl),
      jsonEncode(store.toJson()),
    );
  }
}
