import 'dart:convert';

import 'package:characters/characters.dart';

/// Desktop-compatible personal project and chat placement record.
class MeGroups {
  static const tag = 'buzz-me-kit-v1';
  final List<Map<String, dynamic>> groups;
  final Map<String, String> assignments;
  final List<String> channels;

  const MeGroups({
    this.groups = const [],
    this.assignments = const {},
    this.channels = const [],
  });

  static String coordinate(String community) =>
      '$tag:${Uri.encodeComponent(community)}:groups:me';

  factory MeGroups.decode(String plaintext, String community) {
    if (utf8.encode(plaintext).length > 16384) {
      throw const FormatException('Solo storage is full');
    }
    final raw = jsonDecode(plaintext) as Map<String, dynamic>;
    final value = raw['value'] as Map<String, dynamic>;
    if (raw['version'] != 1 ||
        raw['community'] != community ||
        raw['deleted'] is! bool ||
        value['type'] != 'groups' ||
        value['id'] != 'me') {
      throw const FormatException('Unsupported Solo record');
    }
    final groups = (value['groups'] as List)
        .map((g) => Map<String, dynamic>.from(g as Map))
        .toList();
    final assignments = Map<String, String>.from(value['assignments'] as Map);
    final channels = List<String>.from(value['channels'] as List? ?? []);
    final identifier = RegExp(r'^[a-zA-Z0-9_-]{1,128}$');
    final channelId = RegExp(
      r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$',
    );
    final ids = groups.map((g) => g['id']).toSet();
    if (groups.length > 100 ||
        ids.length != groups.length ||
        groups.any(
          (g) =>
              g['id'] is! String ||
              !identifier.hasMatch(g['id'] as String) ||
              g['name'] is! String ||
              (g['name'] as String).trim().isEmpty ||
              (g['name'] as String).characters.length > 120 ||
              g['defaultTemplateId'] is! String ||
              (g['defaultTemplateId'] != '' &&
                  !identifier.hasMatch(g['defaultTemplateId'] as String)),
        ) ||
        assignments.length > 1000 ||
        assignments.entries.any(
          (e) => !channelId.hasMatch(e.key) || !ids.contains(e.value),
        ) ||
        channels.length > 1000 ||
        channels.toSet().length != channels.length ||
        channels.any((id) => !channelId.hasMatch(id))) {
      throw const FormatException('Invalid Solo projects');
    }
    if (raw['deleted'] == true) return const MeGroups();
    return MeGroups(
      groups: groups,
      assignments: assignments,
      channels: channels,
    );
  }

  String encode(String community) {
    final result = jsonEncode({
      'version': 1,
      'community': community,
      'deleted': false,
      'value': {
        'type': 'groups',
        'id': 'me',
        'groups': groups,
        'assignments': assignments,
        'channels': channels,
      },
    });
    MeGroups.decode(result, community);
    return result;
  }

  MeGroups place(String channel, {String? group}) {
    if (group != null && !groups.any((g) => g['id'] == group)) {
      throw StateError('This project changed. Refresh and try again.');
    }
    return MeGroups(
      groups: groups,
      assignments: {...assignments, channel: ?group},
      channels: [...channels, if (!channels.contains(channel)) channel],
    );
  }
}
