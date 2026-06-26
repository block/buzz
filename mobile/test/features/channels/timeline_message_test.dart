import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/relay/relay.dart';

NostrEvent _textMsg({
  required String id,
  String pubkey = 'alice',
  String content = 'hello',
  int createdAt = 1000,
  List<List<String>>? extraTags,
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.streamMessage,
  tags: [
    ['h', 'ch1'],
    ...?extraTags,
  ],
  content: content,
  sig: '',
);

/// A reply message with proper root/reply e-tag markers.
NostrEvent _replyMsg({
  required String id,
  required String parentId,
  String? rootId,
  String pubkey = 'alice',
  String content = 'reply',
  int createdAt = 2000,
  List<List<String>> extraTags = const [],
}) {
  final root = rootId ?? parentId;
  final eTags = parentId == root
      ? [
          ['e', root, '', 'reply'],
        ]
      : [
          ['e', root, '', 'root'],
          ['e', parentId, '', 'reply'],
        ];
  return _textMsg(
    id: id,
    pubkey: pubkey,
    content: content,
    createdAt: createdAt,
    extraTags: [...eTags, ...extraTags],
  );
}

NostrEvent _systemMsg({
  required String id,
  required Map<String, dynamic> payload,
  int createdAt = 1000,
}) => NostrEvent(
  id: id,
  pubkey: 'relay',
  createdAt: createdAt,
  kind: EventKind.systemMessage,
  tags: [
    ['h', 'ch1'],
  ],
  content: jsonEncode(payload),
  sig: '',
);

NostrEvent _deletion({
  required String id,
  required List<String> targets,
  int createdAt = 2000,
}) => NostrEvent(
  id: id,
  pubkey: 'alice',
  createdAt: createdAt,
  kind: EventKind.deletion,
  tags: [
    ['h', 'ch1'],
    for (final t in targets) ['e', t],
  ],
  content: '',
  sig: '',
);

NostrEvent _nip29Deletion({
  required String id,
  required List<String> targets,
  int createdAt = 2000,
}) => NostrEvent(
  id: id,
  pubkey: 'alice',
  createdAt: createdAt,
  kind: EventKind.nip29DeleteEvent,
  tags: [
    ['h', 'ch1'],
    for (final t in targets) ['e', t],
  ],
  content: '',
  sig: '',
);

NostrEvent _edit({
  required String id,
  required String targetId,
  required String content,
  int createdAt = 2000,
  List<List<String>> extraTags = const [],
}) => NostrEvent(
  id: id,
  pubkey: 'alice',
  createdAt: createdAt,
  kind: EventKind.streamMessageEdit,
  tags: [
    ['h', 'ch1'],
    ['e', targetId],
    ...extraTags,
  ],
  content: content,
  sig: '',
);

NostrEvent _reaction({
  required String id,
  required String targetId,
  int createdAt = 2000,
  String content = '👍',
  List<List<String>> extraTags = const [],
}) => NostrEvent(
  id: id,
  pubkey: 'bob',
  createdAt: createdAt,
  kind: EventKind.reaction,
  tags: [
    ['h', 'ch1'],
    ['e', targetId],
    ...extraTags,
  ],
  content: content,
  sig: '',
);

void main() {
  group('SystemEvent.fromContent', () {
    test('parses all known event types', () {
      final types = {
        'member_joined': SystemEventType.memberJoined,
        'member_left': SystemEventType.memberLeft,
        'member_removed': SystemEventType.memberRemoved,
        'topic_changed': SystemEventType.topicChanged,
        'purpose_changed': SystemEventType.purposeChanged,
        'channel_created': SystemEventType.channelCreated,
        'channel_archived': SystemEventType.channelArchived,
        'channel_unarchived': SystemEventType.channelUnarchived,
      };

      for (final entry in types.entries) {
        final event = SystemEvent.fromContent(
          jsonEncode({'type': entry.key, 'actor': 'pk1'}),
        );
        expect(event, isNotNull, reason: 'Failed for ${entry.key}');
        expect(event!.type, entry.value);
        expect(event.actorPubkey, 'pk1');
      }
    });

    test('returns null for unknown type', () {
      final event = SystemEvent.fromContent(
        jsonEncode({'type': 'unknown_type'}),
      );
      expect(event, isNull);
    });

    test('returns null for invalid JSON', () {
      expect(SystemEvent.fromContent('not json'), isNull);
    });

    test('returns null for wrong JSON field types', () {
      expect(
        SystemEvent.fromContent(
          jsonEncode({
            'type': ['member_joined'],
            'actor': 123,
          }),
        ),
        isNull,
      );
    });

    test('parses target, topic, and purpose fields', () {
      final event = SystemEvent.fromContent(
        jsonEncode({
          'type': 'topic_changed',
          'actor': 'pk1',
          'target': 'pk2',
          'topic': 'New topic',
          'purpose': 'New purpose',
        }),
      );

      expect(event, isNotNull);
      expect(event!.targetPubkey, 'pk2');
      expect(event.topic, 'New topic');
      expect(event.purpose, 'New purpose');
    });
  });

  group('SystemEvent.describe', () {
    String resolve(String? pk) => pk == 'pk1' ? 'Alice' : 'Bob';

    test('member_joined self', () {
      final event = SystemEvent(
        type: SystemEventType.memberJoined,
        actorPubkey: 'pk1',
        targetPubkey: 'pk1',
      );
      expect(event.describe(resolve), 'Alice joined the channel');
    });

    test('member_joined by other', () {
      final event = SystemEvent(
        type: SystemEventType.memberJoined,
        actorPubkey: 'pk1',
        targetPubkey: 'pk2',
      );
      expect(event.describe(resolve), 'Alice added Bob to the channel');
    });

    test('member_left', () {
      final event = SystemEvent(
        type: SystemEventType.memberLeft,
        actorPubkey: 'pk1',
      );
      expect(event.describe(resolve), 'Alice left the channel');
    });

    test('member_removed', () {
      final event = SystemEvent(
        type: SystemEventType.memberRemoved,
        actorPubkey: 'pk1',
        targetPubkey: 'pk2',
      );
      expect(event.describe(resolve), 'Alice removed Bob from the channel');
    });

    test('topic_changed', () {
      final event = SystemEvent(
        type: SystemEventType.topicChanged,
        actorPubkey: 'pk1',
        topic: 'Release v2',
      );
      expect(
        event.describe(resolve),
        'Alice changed the topic to "Release v2"',
      );
    });

    test('purpose_changed', () {
      final event = SystemEvent(
        type: SystemEventType.purposeChanged,
        actorPubkey: 'pk1',
        purpose: 'Daily standups',
      );
      expect(
        event.describe(resolve),
        'Alice changed the purpose to "Daily standups"',
      );
    });

    test('channel_created', () {
      final event = SystemEvent(
        type: SystemEventType.channelCreated,
        actorPubkey: 'pk1',
      );
      expect(event.describe(resolve), 'Alice created this channel');
    });

    test('channel_archived', () {
      final event = SystemEvent(
        type: SystemEventType.channelArchived,
        actorPubkey: 'pk1',
      );
      expect(event.describe(resolve), 'Alice archived this channel');
    });

    test('channel_unarchived', () {
      final event = SystemEvent(
        type: SystemEventType.channelUnarchived,
        actorPubkey: 'pk1',
      );
      expect(event.describe(resolve), 'Alice unarchived this channel');
    });
  });

  group('formatTimeline', () {
    test('passes through text messages', () {
      final events = [
        _textMsg(id: 'a', content: 'hello'),
        _textMsg(id: 'b', content: 'world', createdAt: 1100),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(2));
      expect(result[0].content, 'hello');
      expect(result[1].content, 'world');
      expect(result[0].isSystem, false);
      expect(result[0].edited, false);
    });

    test('filters deleted messages', () {
      final events = [
        _textMsg(id: 'a', content: 'keep'),
        _textMsg(id: 'b', content: 'delete', createdAt: 1100),
        _deletion(id: 'd1', targets: ['b']),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].content, 'keep');
    });

    test('filters messages deleted via kind:9005 (Buzz-native)', () {
      // Agents emit kind:9005 deletes via the CLI. Mobile must mirror desktop
      // and treat 9005 as a deletion marker, otherwise agent-deleted messages
      // stay rendered until manual refresh.
      final events = [
        _textMsg(id: 'a', content: 'keep'),
        _textMsg(id: 'b', content: 'delete', createdAt: 1100),
        _nip29Deletion(id: 'd1', targets: ['b']),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].content, 'keep');
    });

    test('applies edits', () {
      final events = [
        _textMsg(id: 'a', content: 'original'),
        _edit(id: 'e1', targetId: 'a', content: 'edited'),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].content, 'edited');
      expect(result[0].edited, true);
    });

    test('applies edit tags for custom emoji rendering', () {
      final events = [
        _textMsg(
          id: 'a',
          content: 'original :old:',
          extraTags: [
            ['emoji', 'old', 'https://relay.example/old.png'],
          ],
        ),
        _edit(
          id: 'e1',
          targetId: 'a',
          content: 'edited :new:',
          extraTags: [
            ['emoji', 'new', 'https://relay.example/new.png'],
          ],
        ),
      ];

      final result = formatTimeline(events);
      expect(result.single.content, 'edited :new:');
      expect(
        result.single.tags.any(
          (tag) =>
              tag.length >= 3 &&
              tag[0] == 'emoji' &&
              tag[1] == 'new' &&
              tag[2] == 'https://relay.example/new.png',
        ),
        isTrue,
      );
      expect(
        result.single.tags.any(
          (tag) =>
              tag.length >= 3 &&
              tag[0] == 'emoji' &&
              tag[1] == 'old' &&
              tag[2] == 'https://relay.example/old.png',
        ),
        isFalse,
      );
    });

    test('preserves custom emoji reaction image URL from emoji tag', () {
      final messages = formatTimeline([
        _textMsg(id: 'm1'),
        _reaction(
          id: 'r1',
          targetId: 'm1',
          content: ':shipit:',
          extraTags: [
            ['emoji', 'shipit', 'https://relay.example/shipit.png'],
          ],
        ),
      ], currentPubkey: 'bob');

      final reaction = messages.single.reactions.single;
      expect(reaction.emoji, ':shipit:');
      expect(reaction.emojiUrl, 'https://relay.example/shipit.png');
      expect(reaction.reactedByCurrentUser, isTrue);
    });

    test('latest edit wins', () {
      final events = [
        _textMsg(id: 'a', content: 'v1'),
        _edit(id: 'e1', targetId: 'a', content: 'v2', createdAt: 2000),
        _edit(id: 'e2', targetId: 'a', content: 'v3', createdAt: 3000),
      ];

      final result = formatTimeline(events);
      expect(result[0].content, 'v3');
    });

    test('deleted edit is ignored', () {
      final events = [
        _textMsg(id: 'a', content: 'original'),
        _edit(id: 'e1', targetId: 'a', content: 'edited'),
        _deletion(id: 'd1', targets: ['e1']),
      ];

      final result = formatTimeline(events);
      expect(result[0].content, 'original');
      expect(result[0].edited, false);
    });

    test('edit of deleted message is ignored', () {
      final events = [
        _textMsg(id: 'a', content: 'original'),
        _edit(id: 'e1', targetId: 'a', content: 'edited'),
        _deletion(id: 'd1', targets: ['a']),
      ];

      final result = formatTimeline(events);
      expect(result, isEmpty);
    });

    test('system messages are parsed', () {
      final events = [
        _systemMsg(
          id: 's1',
          payload: {'type': 'channel_created', 'actor': 'pk1'},
        ),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].isSystem, true);
      expect(result[0].systemEvent, isNotNull);
      expect(result[0].systemEvent!.type, SystemEventType.channelCreated);
    });

    test('unknown system messages are dropped', () {
      final events = [
        _systemMsg(id: 's1', payload: {'type': 'unknown'}),
      ];

      final result = formatTimeline(events);
      expect(result, isEmpty);
    });

    test('malformed system messages are skipped safely', () {
      final events = [
        _systemMsg(
          id: 's1',
          payload: {
            'type': ['member_joined'],
            'actor': 123,
          },
        ),
        _textMsg(id: 'a', content: 'hello', createdAt: 1100),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].content, 'hello');
    });

    test('reactions and typing indicators are filtered out', () {
      final events = [
        _textMsg(id: 'a', content: 'hello'),
        _reaction(id: 'r1', targetId: 'a'),
        NostrEvent(
          id: 'typing1',
          pubkey: 'bob',
          createdAt: 1000,
          kind: EventKind.typingIndicator,
          tags: [
            ['h', 'ch1'],
          ],
          content: '',
          sig: '',
        ),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].content, 'hello');
    });

    test('preserves chronological order', () {
      final events = [
        _textMsg(id: 'a', content: 'first', createdAt: 1000),
        _systemMsg(
          id: 's1',
          payload: {'type': 'member_joined', 'actor': 'pk1', 'target': 'pk1'},
          createdAt: 1100,
        ),
        _textMsg(id: 'b', content: 'second', createdAt: 1200),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(3));
      expect(result[0].content, 'first');
      expect(result[1].isSystem, true);
      expect(result[2].content, 'second');
    });

    test('streamMessageV2 events are included', () {
      final events = [
        NostrEvent(
          id: 'v2msg',
          pubkey: 'alice',
          createdAt: 1000,
          kind: EventKind.streamMessageV2,
          tags: [
            ['h', 'ch1'],
          ],
          content: 'legacy message',
          sig: '',
        ),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].content, 'legacy message');
    });

    test('streamMessageDiff events are included', () {
      final events = [
        NostrEvent(
          id: 'diff1',
          pubkey: 'alice',
          createdAt: 1000,
          kind: EventKind.streamMessageDiff,
          tags: [
            ['h', 'ch1'],
          ],
          content: '```diff\n-old\n+new\n```',
          sig: '',
        ),
      ];

      final result = formatTimeline(events);
      expect(result, hasLength(1));
      expect(result[0].content, contains('diff'));
    });
  });

  group('NostrEvent.threadReference', () {
    test('returns nulls for top-level message (no e-tags)', () {
      final event = _textMsg(id: 'a');
      final ref = event.threadReference;
      expect(ref.parentId, isNull);
      expect(ref.rootId, isNull);
    });

    test('returns nulls for e-tags without root/reply markers', () {
      final event = _textMsg(
        id: 'a',
        extraTags: [
          ['e', 'target'],
        ],
      );
      final ref = event.threadReference;
      expect(ref.parentId, isNull);
      expect(ref.rootId, isNull);
    });

    test('parses direct reply to root (single reply marker)', () {
      final event = _textMsg(
        id: 'reply1',
        extraTags: [
          ['e', 'root1', '', 'reply'],
        ],
      );
      final ref = event.threadReference;
      expect(ref.parentId, 'root1');
      expect(ref.rootId, 'root1');
    });

    test('parses nested reply (root + reply markers)', () {
      final event = _textMsg(
        id: 'reply2',
        extraTags: [
          ['e', 'root1', '', 'root'],
          ['e', 'parent1', '', 'reply'],
        ],
      );
      final ref = event.threadReference;
      expect(ref.parentId, 'parent1');
      expect(ref.rootId, 'root1');
    });

    test('last reply tag wins when multiple exist', () {
      final event = _textMsg(
        id: 'reply3',
        extraTags: [
          ['e', 'root1', '', 'root'],
          ['e', 'old_parent', '', 'reply'],
          ['e', 'new_parent', '', 'reply'],
        ],
      );
      final ref = event.threadReference;
      expect(ref.parentId, 'new_parent');
      expect(ref.rootId, 'root1');
    });
  });

  group('formatTimeline threading', () {
    test('root messages have null parentId and rootId', () {
      final events = [_textMsg(id: 'a')];
      final result = formatTimeline(events);
      expect(result[0].parentId, isNull);
      expect(result[0].rootId, isNull);
    });

    test('reply messages carry parentId and rootId', () {
      final events = [
        _textMsg(id: 'root1'),
        _replyMsg(id: 'r1', parentId: 'root1', createdAt: 2000),
      ];
      final result = formatTimeline(events);
      final reply = result.firstWhere((m) => m.id == 'r1');
      expect(reply.parentId, 'root1');
      expect(reply.rootId, 'root1');
    });

    test('nested reply has distinct parentId and rootId', () {
      final events = [
        _textMsg(id: 'root1'),
        _replyMsg(id: 'r1', parentId: 'root1', createdAt: 2000),
        _replyMsg(id: 'r2', parentId: 'r1', rootId: 'root1', createdAt: 3000),
      ];
      final result = formatTimeline(events);
      final nested = result.firstWhere((m) => m.id == 'r2');
      expect(nested.parentId, 'r1');
      expect(nested.rootId, 'root1');
    });
  });

  group('buildMainTimelineEntries', () {
    test('returns only root messages', () {
      final messages = formatTimeline([
        _textMsg(id: 'a', createdAt: 1000),
        _replyMsg(id: 'r1', parentId: 'a', createdAt: 2000),
        _textMsg(id: 'b', createdAt: 3000),
      ]);

      final entries = buildMainTimelineEntries(messages);
      expect(entries, hasLength(2));
      expect(entries[0].message.id, 'a');
      expect(entries[1].message.id, 'b');
    });

    test('includes broadcast replies in the main timeline', () {
      final messages = formatTimeline([
        _textMsg(id: 'a', createdAt: 1000),
        _replyMsg(id: 'hidden', parentId: 'a', createdAt: 2000),
        _replyMsg(
          id: 'broadcast',
          parentId: 'a',
          createdAt: 3000,
          extraTags: const [
            ['broadcast', '1'],
          ],
        ),
      ]);

      final entries = buildMainTimelineEntries(messages);
      expect(entries.map((entry) => entry.message.id), ['a', 'broadcast']);
    });

    test('root without replies has null summary', () {
      final messages = formatTimeline([_textMsg(id: 'a')]);
      final entries = buildMainTimelineEntries(messages);
      expect(entries[0].summary, isNull);
    });

    test('root with replies has summary with correct count', () {
      final messages = formatTimeline([
        _textMsg(id: 'a', createdAt: 1000),
        _replyMsg(id: 'r1', parentId: 'a', pubkey: 'bob', createdAt: 2000),
        _replyMsg(id: 'r2', parentId: 'a', pubkey: 'carol', createdAt: 3000),
      ]);

      final entries = buildMainTimelineEntries(messages);
      expect(entries, hasLength(1));
      expect(entries[0].summary, isNotNull);
      expect(entries[0].summary!.replyCount, 2);
      expect(entries[0].summary!.threadHeadId, 'a');
    });

    test('summary counts only direct children, not nested replies', () {
      final messages = formatTimeline([
        _textMsg(id: 'a', createdAt: 1000),
        _replyMsg(id: 'r1', parentId: 'a', createdAt: 2000),
        _replyMsg(id: 'r2', parentId: 'r1', rootId: 'a', createdAt: 3000),
      ]);

      final entries = buildMainTimelineEntries(messages);
      // Only r1 is a direct child of a; r2 is a child of r1.
      expect(entries[0].summary!.replyCount, 1);
    });

    test('summary has up to 3 unique participant pubkeys', () {
      final messages = formatTimeline([
        _textMsg(id: 'a', createdAt: 1000),
        _replyMsg(id: 'r1', parentId: 'a', pubkey: 'bob', createdAt: 2000),
        _replyMsg(id: 'r2', parentId: 'a', pubkey: 'carol', createdAt: 3000),
        _replyMsg(id: 'r3', parentId: 'a', pubkey: 'bob', createdAt: 4000),
        _replyMsg(id: 'r4', parentId: 'a', pubkey: 'dave', createdAt: 5000),
        _replyMsg(id: 'r5', parentId: 'a', pubkey: 'eve', createdAt: 6000),
      ]);

      final entries = buildMainTimelineEntries(messages);
      final participants = entries[0].summary!.participantPubkeys;
      expect(participants, hasLength(3));
      // Most recent unique: eve, dave, bob (r5, r4, r3 — carol skipped
      // because bob at r3 comes before carol at r2 when walking backwards).
      // Reversed to chronological order.
      expect(participants, ['bob', 'dave', 'eve']);
    });

    test('system messages remain in entries (parentId is null)', () {
      final events = [
        _systemMsg(
          id: 's1',
          payload: {'type': 'channel_created', 'actor': 'pk1'},
          createdAt: 500,
        ),
        _textMsg(id: 'a', createdAt: 1000),
      ];
      final messages = formatTimeline(events);
      final entries = buildMainTimelineEntries(messages);
      expect(entries, hasLength(2));
      expect(entries[0].message.isSystem, true);
    });

    test('empty input returns empty', () {
      expect(buildMainTimelineEntries([]), isEmpty);
    });
  });
}
