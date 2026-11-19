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

  /// Applies fields edited since [base] to the current relay snapshot.
  ChannelSectionStore applyLocalEdits(
    ChannelSectionStore base,
    ChannelSectionStore remote,
  ) {
    final before = {for (final section in base.sections) section.id: section};
    final local = {for (final section in sections) section.id: section};
    final merged = {for (final section in remote.sections) section.id: section};
    for (final id in before.keys) {
      if (!local.containsKey(id)) merged.remove(id);
    }
    for (final edited in sections) {
      final previous = before[edited.id];
      if (previous == null) {
        merged[edited.id] = edited;
        continue;
      }
      final nameChanged = edited.name != previous.name;
      final iconChanged = edited.icon != previous.icon;
      final orderChanged = edited.order != previous.order;
      if (!(nameChanged || iconChanged || orderChanged)) continue;
      final current = merged[edited.id] ?? previous;
      merged[edited.id] = ChannelSection(
        id: edited.id,
        name: nameChanged ? edited.name : current.name,
        icon: iconChanged ? edited.icon : current.icon,
        order: orderChanged ? edited.order : current.order,
      );
    }

    final mergedAssignments = Map<String, String>.of(remote.assignments);
    for (final channel in {...base.assignments.keys, ...assignments.keys}) {
      final section = assignments[channel];
      if (section == base.assignments[channel]) continue;
      if (section == null) {
        mergedAssignments.remove(channel);
      } else {
        mergedAssignments[channel] = section;
      }
    }
    mergedAssignments.removeWhere((_, section) => !merged.containsKey(section));
    return ChannelSectionStore(
      sections: merged.values.toList(),
      assignments: mergedAssignments,
    );
  }
}

class ChannelSectionsStorage {
  final SharedPreferences _prefs;

  ChannelSectionsStorage(this._prefs);

  ChannelSectionStore read(String pubkey, String relayUrl) =>
      readWithPending(pubkey, relayUrl).store;

  /// The cached store and its pre-edit base are persisted in the same record.
  ({ChannelSectionStore store, ChannelSectionStore? pendingBase})
  readWithPending(String pubkey, String relayUrl) {
    // The shared entry may hold another community's groups, and the relay
    // copy is authoritative, so drop it rather than let one community claim it.
    _prefs.remove(legacyChannelSectionsKey(pubkey));
    final raw = _prefs.getString(channelSectionsKey(pubkey, relayUrl));
    const empty = (store: ChannelSectionStore(), pendingBase: null);
    if (raw == null || raw.isEmpty) return empty;

    try {
      final parsed = jsonDecode(raw);
      if (parsed is! Map<String, dynamic> || parsed['version'] != 1) {
        return empty;
      }
      final pending = parsed['pending_base'];
      return (
        store: ChannelSectionStore.fromJson(parsed),
        pendingBase: pending is Map<String, dynamic>
            ? ChannelSectionStore.fromJson(pending)
            : null,
      );
    } catch (_) {
      return empty;
    }
  }

  void write(
    String pubkey,
    String relayUrl,
    ChannelSectionStore store, {
    ChannelSectionStore? pendingBase,
  }) {
    _prefs.setString(
      channelSectionsKey(pubkey, relayUrl),
      jsonEncode({
        ...store.toJson(),
        if (pendingBase != null) 'pending_base': pendingBase.toJson(),
      }),
    );
  }
}
