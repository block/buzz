import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

String channelSectionsKey(String pubkey) =>
    'sprout.channel-sections.v1:$pubkey';

class ChannelSection {
  final String id;
  final String name;
  final int order;

  const ChannelSection({
    required this.id,
    required this.name,
    required this.order,
  });

  Map<String, dynamic> toJson() => {'id': id, 'name': name, 'order': order};

  factory ChannelSection.fromJson(Map<String, dynamic> json) => ChannelSection(
    id: json['id'] as String,
    name: json['name'] as String,
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

  ChannelSectionStore read(String pubkey) {
    final raw = _prefs.getString(channelSectionsKey(pubkey));
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

  void write(String pubkey, ChannelSectionStore store) {
    _prefs.setString(channelSectionsKey(pubkey), jsonEncode(store.toJson()));
  }
}
