import 'dart:convert';
import 'dart:io';

import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/features/channels/channel_sections/workspace_layout.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:flutter_test/flutter_test.dart';

/// Vectors produced by desktop's own sidebar code (block/buzz-app #3791):
/// the record decoder, legacy merge, import fallback and section order.
void main() {
  final fixture =
      jsonDecode(
            File(
              'test/fixtures/workspace_layout_desktop.json',
            ).readAsStringSync(),
          )
          as Map<String, dynamic>;

  for (final raw in fixture['cases'] as List) {
    final c = raw as Map<String, dynamic>;
    test('matches desktop: ${c['name']}', () {
      final solo = c['solo'] as Map<String, dynamic>?;
      final layout = effectiveWorkspaceLayout(
        unified: c.containsKey('unified')
            ? WorkspaceRecord.fromJson(c['unified'])
            : null,
        team: ChannelSectionStore.fromJson(c['team'] as Map<String, dynamic>),
        solo: solo == null
            ? null
            : MeGroups(
                groups: [
                  for (final group in solo['groups'] as List)
                    Map<String, dynamic>.from(group as Map),
                ],
                assignments: Map<String, String>.from(
                  solo['assignments'] as Map,
                ),
              ),
      );
      final expected = c['expected'] as Map<String, dynamic>;

      expect([
        for (final section in layout.sections)
          {
            'id': section.id,
            'name': section.name,
            if (section.icon != null) 'icon': section.icon,
          },
      ], expected['sections']);
      expect(layout.assignments, expected['assignments']);
      expect(layout.sectionKeys, expected['sectionKeys']);
    });
  }

  for (final raw in fixture['invalid'] as List) {
    final c = raw as Map<String, dynamic>;
    test('rejects what desktop rejects: ${c['name']}', () {
      expect(
        () => WorkspaceRecord.fromJson(c['unified']),
        throwsFormatException,
      );
    });
  }
}
