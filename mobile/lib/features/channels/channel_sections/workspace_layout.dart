import 'dart:collection';
import 'dart:convert';

import '../../../shared/me/me_groups.dart';
import 'channel_sections_storage.dart';

/// The unified desktop sidebar layout (buzz-app `channel-workspaces`), read
/// only. Desktop owns writes; mobile renders the same sections in the same
/// order. Keep these rules in step with buzz-app's `sidebar-preferences.ts`,
/// `sidebar-registers.ts` and `sidebar-sections.ts`.
const workspaceCoordinate = 'channel-workspaces';

const _maxWorkspaces = 200;
const _maxWorkspaceAssignments = 2000;

/// Desktop's home sections after Starred, in their default order.
const homeSidebarSections = ['solo', 'channels', 'forums', 'dms'];
const _pagesSection = 'pages';

class WorkspaceSection {
  final String id;
  final String name;

  /// A Unicode emoji, or a `:shortcode:` naming a community custom emoji.
  final String? icon;
  final num order;

  /// The legacy Team section this workspace projects, whose ID can differ
  /// from [id] when a Solo project shares it.
  final String? teamId;

  const WorkspaceSection({
    required this.id,
    required this.name,
    this.icon,
    required this.order,
    this.teamId,
  });
}

/// The workspaces, filings and section order the desktop sidebar renders.
class WorkspaceLayout {
  /// Sorted by order, then id.
  final List<WorkspaceSection> sections;

  /// Channel id to workspace id. Every target is a listed workspace.
  final Map<String, String> assignments;

  /// The saved section order; empty means desktop's default order.
  final List<String> sectionOrder;

  const WorkspaceLayout({
    this.sections = const [],
    this.assignments = const {},
    this.sectionOrder = const [],
  });

  /// Sorts and filters raw sections and filings into a renderable layout.
  factory WorkspaceLayout.project(
    Iterable<WorkspaceSection> sections,
    Map<String, String> assignments, [
    List<String> sectionOrder = const [],
  ]) {
    final sorted = sections.toList()
      ..sort((a, b) {
        final order = a.order.compareTo(b.order);
        return order != 0 ? order : a.id.compareTo(b.id);
      });
    final ids = {for (final section in sorted) section.id};
    return WorkspaceLayout(
      sections: List.unmodifiable(sorted),
      assignments: Map.unmodifiable({
        for (final entry in assignments.entries)
          if (ids.contains(entry.value)) entry.key: entry.value,
      }),
      sectionOrder: List.unmodifiable(sectionOrder),
    );
  }

  WorkspaceSection? section(String id) {
    for (final section in sections) {
      if (section.id == id) return section;
    }
    return null;
  }

  /// Section keys in render order: `starred`, `group:<id>` for each
  /// workspace, and the [homeSidebarSections]. Ports `workspaceSectionOrder`.
  List<String> get sectionKeys {
    final defaults = [
      'starred',
      for (final section in sections) 'group:${section.id}',
      ...homeSidebarSections,
    ];
    if (sectionOrder.isEmpty) return defaults;
    final available = LinkedHashSet.of(defaults);
    final order = [
      for (final key in sectionOrder)
        if (available.remove(key)) key,
    ];
    if (available.remove('starred')) order.insert(0, 'starred');
    // Workspaces added since the last reorder follow the existing workspaces.
    final added = [
      for (final key in defaults)
        if (key.startsWith('group:') && available.contains(key)) key,
    ];
    available.removeAll(added);
    final lastWorkspace = order.lastIndexWhere((k) => k.startsWith('group:'));
    order.insertAll(
      lastWorkspace >= 0 ? lastWorkspace + 1 : order.indexOf('starred') + 1,
      added,
    );
    return [...order, ...available];
  }

  /// The section a conversation renders in: Starred first, then its
  /// workspace, then [home] (one of [homeSidebarSections]).
  String sectionKeyFor(
    String channelId, {
    required bool starred,
    required String home,
  }) {
    if (starred) return 'starred';
    final workspace = assignments[channelId];
    return workspace == null ? home : 'group:$workspace';
  }
}

/// The sort and filter group key desktop stores for a section key.
String sidebarGroupKey(String sectionKey) => sectionKey.startsWith('group:')
    ? 'section:${sectionKey.substring(6)}'
    : sectionKey;

/// A decoded `channel-workspaces` head. Any invalid field rejects the whole
/// record, as desktop does.
class WorkspaceRecord {
  final List<WorkspaceSection> _sections;

  /// Raw filings, including any whose workspace no longer exists: those still
  /// block a legacy filing of the same channel from being imported.
  final Map<String, String> _assignments;
  final List<String> _sectionOrder;
  final Set<String> _removedSections;
  final Set<String> _unfiled;
  final bool _legacyImportClosed;

  const WorkspaceRecord._(
    this._sections,
    this._assignments,
    this._sectionOrder,
    this._removedSections,
    this._unfiled,
    this._legacyImportClosed,
  );

  static WorkspaceRecord decode(String plaintext) =>
      WorkspaceRecord.fromJson(jsonDecode(plaintext));

  factory WorkspaceRecord.fromJson(Object? value) {
    final data = _expand(_object(value));
    final rawSections = data['sections'];
    if (data['version'] != 1 ||
        rawSections is! List ||
        rawSections.length > _maxWorkspaces) {
      throw const FormatException('Unsupported workspace record');
    }
    final ids = <String>{};
    final sections = [
      for (final raw in rawSections) _section(_object(raw), ids),
    ];
    final rawAssignments = _object(data['assignments']);
    if (rawAssignments.length > _maxWorkspaceAssignments) {
      throw const FormatException('Workspace filing budget exceeded');
    }
    final assignments = {
      for (final entry in rawAssignments.entries)
        _text(entry.key): _text(entry.value),
    };
    final sectionOrder = _sectionOrderOf(data['sectionOrder']);
    final removed = _tombstones(data['removedSections']);
    final unfiled = _tombstones(data['unfiled']);
    final closed = data['legacyImportClosed'];
    if (closed != null &&
        (closed != true ||
            data.containsKey('removedSections') ||
            data.containsKey('unfiled') ||
            data.containsKey('unifiedOnly'))) {
      throw const FormatException('Invalid workspace legacy import state');
    }
    return WorkspaceRecord._(
      sections,
      assignments,
      sectionOrder,
      removed,
      unfiled,
      closed == true,
    );
  }

  /// The layout desktop renders: this record plus any legacy workspaces and
  /// filings it has never seen. Ports `importWorkspaceFallback`.
  WorkspaceLayout resolve(WorkspaceLayout? legacy) {
    final sections = _sections.toList();
    final assignments = Map.of(_assignments);
    if (!_legacyImportClosed && legacy != null && legacy.sections.isNotEmpty) {
      final live = {for (final section in sections) section.id};
      var order =
          sections.fold<num>(-1, (top, s) => s.order > top ? s.order : top) + 1;
      for (final section in legacy.sections) {
        if (sections.length == _maxWorkspaces) break;
        if (live.contains(section.id) ||
            _removedSections.contains(section.id)) {
          continue;
        }
        live.add(section.id);
        sections.add(
          WorkspaceSection(
            id: section.id,
            name: section.name,
            icon: section.icon,
            order: order++,
          ),
        );
      }
      for (final entry in legacy.assignments.entries) {
        if (assignments.length == _maxWorkspaceAssignments) break;
        if (assignments.containsKey(entry.key) ||
            _unfiled.contains(entry.key) ||
            !live.contains(entry.value)) {
          continue;
        }
        assignments[entry.key] = entry.value;
      }
    }
    return WorkspaceLayout.project(sections, assignments, _sectionOrder);
  }

  static Map<String, dynamic> _expand(Map<String, dynamic> data) {
    final filings = data['filings'];
    if (filings == null) {
      // Desktop never writes register metadata into this record; refuse to
      // guess at one rather than render a projection desktop would not.
      if (data.containsKey('packedUnfiled') || data.containsKey('meta')) {
        throw const FormatException('Unsupported workspace record');
      }
      return data;
    }
    if (data.containsKey('assignments') || data.containsKey('meta')) {
      throw const FormatException('Invalid compact workspace filings');
    }
    final record = Map<String, dynamic>.of(data)
      ..remove('filings')
      ..remove('packedUnfiled');
    final packedUnfiled = data['packedUnfiled'];
    if (packedUnfiled != null) {
      if (record.containsKey('unfiled') || packedUnfiled is! List) {
        throw const FormatException('Invalid compact workspace markers');
      }
      record['unfiled'] = [
        for (final id in packedUnfiled) _text(_unpack(_text(id, 257))),
      ];
    }
    final entries = _object(filings).entries;
    final rawSections = record['sections'];
    if (entries.length > _maxWorkspaces || rawSections is! List) {
      throw const FormatException('Invalid compact workspace filings');
    }
    final workspaceIds = {
      for (final section in rawSections)
        if (section is Map) section['id'],
    };
    final assignments = <String, String>{};
    for (final entry in entries) {
      final rows = entry.value;
      if (!workspaceIds.contains(_text(entry.key)) ||
          rows is! List ||
          rows.isEmpty) {
        throw const FormatException('Invalid compact workspace filings');
      }
      for (final row in rows) {
        final id = _text(_unpack(_text(row, 257)));
        if (id == '__proto__' ||
            assignments.containsKey(id) ||
            assignments.length >= _maxWorkspaceAssignments) {
          throw const FormatException('Invalid compact workspace filings');
        }
        assignments[id] = entry.key;
      }
    }
    return record..['assignments'] = assignments;
  }

  static final _hexId = RegExp(r'^[0-9a-f]{64}$');
  static final _packedId = RegExp(r'^~[A-Za-z0-9+/]{43}$');

  /// Compact filings store a 64-hex id as `~` + unpadded base64 of its bytes,
  /// and escape any other id starting with `~` as `~~`.
  static String _unpack(String id) {
    if (id.startsWith('~~')) return id.substring(1);
    if (!id.startsWith('~') && !_hexId.hasMatch(id)) return id;
    if (!_packedId.hasMatch(id)) {
      throw const FormatException('Invalid compact workspace filing');
    }
    final bytes = base64.decode('${id.substring(1)}=');
    if ('~${base64.encode(bytes).substring(0, 43)}' != id) {
      throw const FormatException('Invalid compact workspace filing');
    }
    return [for (final b in bytes) b.toRadixString(16).padLeft(2, '0')].join();
  }

  static WorkspaceSection _section(Map<String, dynamic> raw, Set<String> ids) {
    final id = _text(raw['id']);
    final order = raw['order'];
    if (!ids.add(id) || order is! num || !order.isFinite) {
      throw const FormatException('Invalid workspace order');
    }
    return WorkspaceSection(
      id: id,
      name: _text(raw['name']),
      icon: raw.containsKey('icon') ? _text(raw['icon'], 128) : null,
      order: order,
    );
  }

  static List<String> _sectionOrderOf(Object? value) {
    const homeKeys = {'starred', ...homeSidebarSections};
    if (value is! List ||
        value.length > _maxWorkspaces + homeKeys.length + 1 ||
        value.toSet().length != value.length ||
        !value.every(
          (key) =>
              key is String &&
              (key == _pagesSection ||
                  homeKeys.contains(key) ||
                  (key.startsWith('group:') &&
                      key.substring(6).trim().isNotEmpty &&
                      key.length <= 262)),
        )) {
      throw const FormatException('Invalid workspace section order');
    }
    return value.cast<String>();
  }

  static Set<String> _tombstones(Object? value) {
    if (value == null) return const {};
    if (value is! List ||
        value.toSet().length != value.length ||
        value.any((id) => _text(id) == '__proto__')) {
      throw const FormatException('Invalid workspace markers');
    }
    return value.cast<String>().toSet();
  }
}

/// The legacy layout desktop merges before its first workspace save: Solo
/// projects first, then team sections. Ports `mergeLegacyWorkspaces`. Throws
/// when a team section cannot be given a distinct id.
WorkspaceLayout legacyWorkspaceLayout(ChannelSectionStore team, MeGroups solo) {
  final teamLayout = WorkspaceLayout.project(
    team.sections.map(
      (s) => WorkspaceSection(
        id: s.id,
        name: s.name,
        icon: s.icon,
        order: s.order,
        teamId: s.id,
      ),
    ),
    team.assignments,
  );
  if (solo.groups.isEmpty && solo.assignments.isEmpty) return teamLayout;
  final soloIds = {for (final group in solo.groups) group['id'] as String};
  final renamed = <String, String>{};
  for (final section in teamLayout.sections) {
    if (!soloIds.contains(section.id)) continue;
    final id = 'legacy-team:${section.id}';
    if (teamLayout.section(id) != null) {
      throw const FormatException('Legacy workspace ids collide');
    }
    renamed[section.id] = id;
  }
  final sections = [
    for (final group in solo.groups)
      (
        id: group['id'] as String,
        name: group['name'] as String,
        icon: null,
        teamId: null,
      ),
    for (final s in teamLayout.sections)
      (id: renamed[s.id] ?? s.id, name: s.name, icon: s.icon, teamId: s.id),
  ];
  return WorkspaceLayout.project(
    [
      for (final (index, s) in sections.indexed)
        WorkspaceSection(
          id: s.id,
          name: s.name,
          icon: s.icon,
          order: index,
          teamId: s.teamId,
        ),
    ],
    {
      for (final entry in teamLayout.assignments.entries)
        entry.key: renamed[entry.value] ?? entry.value,
      ...solo.assignments,
    },
  );
}

/// What desktop renders for this account. [solo] is null while the Solo
/// record is unavailable: desktop then shows team groups only and imports
/// nothing into a unified record.
WorkspaceLayout effectiveWorkspaceLayout({
  required WorkspaceRecord? unified,
  required ChannelSectionStore team,
  required MeGroups? solo,
}) {
  WorkspaceLayout? legacy;
  if (solo != null) {
    try {
      legacy = legacyWorkspaceLayout(team, solo);
    } on FormatException {
      legacy = null;
    }
  }
  if (unified != null) return unified.resolve(legacy);
  return legacy ?? legacyWorkspaceLayout(team, const MeGroups());
}

Map<String, dynamic> _object(Object? value) {
  if (value is Map<String, dynamic>) return value;
  if (value is Map && value.keys.every((key) => key is String)) {
    return value.cast<String, dynamic>();
  }
  throw const FormatException('Expected a JSON object');
}

String _text(Object? value, [int max = 256]) {
  if (value is String && value.trim().isNotEmpty && value.length <= max) {
    return value;
  }
  throw const FormatException('Invalid workspace text');
}
