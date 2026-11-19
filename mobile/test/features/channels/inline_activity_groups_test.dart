import 'package:buzz/features/channels/inline_activity_groups.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'activity runs respect public messages, notices, days and thread roots',
    () {
      TimelineMessage message(
        String id, {
        bool quiet = true,
        bool system = false,
        int day = 1,
        String? root,
      }) => TimelineMessage(
        id: id,
        pubkey: 'agent',
        createdAt: DateTime(2026, 11, day, 12).millisecondsSinceEpoch ~/ 1000,
        content: id,
        isSystem: system,
        rootId: root,
        tags: [
          if (quiet) ['audience', 'agents'],
        ],
      );
      final items = [
        message('a'),
        message('b'),
        message('public', quiet: false),
        message('c'),
        message('notice', system: true),
        message('d'),
        message('next-day', day: 2),
        message('thread', day: 2, root: 'root'),
      ];
      expect(
        groupInlineActivity(
          items,
          (m) => m,
        ).map((g) => g.map((m) => m.id).toList()).toList(),
        [
          ['a', 'b'],
          ['public'],
          ['c'],
          ['notice'],
          ['d'],
          ['next-day'],
          ['thread'],
        ],
      );
      // A link has its own displayed row so the timeline can scroll to the
      // specific update even inside a history taller than the viewport.
      expect(
        groupInlineActivity(
          items,
          (m) => m,
          revealMessageId: 'b',
        ).take(2).map((g) => g.single.id),
        ['a', 'b'],
      );
    },
  );
}
