import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/theme/community_theme_preference.dart';

/// Groups belong to one community, so the cache is keyed by relay as well as
/// identity. One identity can be paired with several communities.
String channelSectionsKey(String pubkey, String relayUrl) =>
    'buzz.channel-sections.v1:$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

/// The earlier key, shared by every community of the identity.
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

  ChannelSectionStore read(String pubkey, String relayUrl) {
    // The shared entry may hold another community's groups, and the relay
    // copy is authoritative, so drop it rather than let one community claim it.
    _prefs.remove(legacyChannelSectionsKey(pubkey));
    final raw = _prefs.getString(channelSectionsKey(pubkey, relayUrl));
    if (raw == null || raw.isEmpty) {
      return const ChannelSectionStore();
    }

    try {
      final parsed = jsonDecode(raw);
      if (parsed is! Map<String, dynamic>) {
        return const ChannelSectionStore();
      }
      if (parsed['version'] != 1) {
        return const ChannelSectionStore();
      }
      return ChannelSectionStore.fromJson(parsed);
    } catch (_) {
      return const ChannelSectionStore();
    }
  }

  void write(String pubkey, String relayUrl, ChannelSectionStore store) {
    _prefs.setString(
      channelSectionsKey(pubkey, relayUrl),
      jsonEncode(store.toJson()),
    );
  }
}
