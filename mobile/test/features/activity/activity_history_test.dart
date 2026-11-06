import 'package:buzz/features/activity/activity_history.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

String id(int n) => n.toRadixString(16).padLeft(64, '0');
NostrEvent event(
  int n, {
  String author = 'other',
  String channel = 'team',
  int kind = 9,
  List<List<String>> tags = const [],
  String? content,
}) => NostrEvent(
  id: id(n),
  pubkey: author,
  createdAt: n,
  kind: kind,
  tags: [
    ['h', channel],
    ...tags,
  ],
  content: content ?? 'message $n',
  sig: '',
);
Channel channel(
  String name, {
  bool solo = false,
  bool dm = false,
  bool archived = false,
}) => Channel(
  id: name,
  name: name,
  channelType: dm ? 'dm' : 'stream',
  visibility: 'private',
  description: '',
  createdBy: 'me',
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
  labels: solo ? ['buzz.solo'] : [],
  archivedAt: archived ? DateTime(2026) : null,
);
ActivityQuery reader(List<NostrEvent> events) =>
    (filters) async => [
      for (final f in filters)
        ...(events
                .where(
                  (e) =>
                      f.kinds.contains(e.kind) &&
                      (f.ids == null || f.ids!.contains(e.id)) &&
                      (f.authors == null || f.authors!.contains(e.pubkey)) &&
                      (f.until == null || e.createdAt <= f.until!) &&
                      (f.since == null || e.createdAt >= f.since!) &&
                      (f.extensions['before_id'] == null ||
                          e.createdAt != f.until ||
                          e.id.compareTo(f.extensions['before_id'] as String) >
                              0) &&
                      f.tags.entries.every(
                        (t) => e.tags.any(
                          (v) =>
                              v.length > 1 &&
                              v[0] == t.key.substring(1) &&
                              t.value.contains(v[1]),
                        ),
                      ),
                )
                .toList()
              ..sort((a, b) => b.createdAt.compareTo(a.createdAt)))
            .take(f.limit),
    ];
void main() {
  test(
    'includes unmentioned participated/followed replies, Solo, and DMs, but not unrelated team traffic',
    () async {
      final events = [
        event(1, author: 'me'),
        event(
          2,
          tags: [
            ['e', id(1), '', 'root'],
            ['e', id(1), '', 'reply'],
          ],
        ),
        event(3),
        event(
          4,
          tags: [
            ['e', id(3), '', 'root'],
            ['e', id(3), '', 'reply'],
          ],
        ),
        event(12, kind: 45001, author: 'me'),
        event(
          13,
          kind: 45003,
          tags: [
            ['e', id(12), '', 'reply'],
          ],
        ),
        event(14, kind: 45001),
        event(
          15,
          kind: 45003,
          tags: [
            ['e', id(14), '', 'reply'],
          ],
        ),
        event(5),
        event(
          6,
          tags: [
            ['e', id(5), '', 'root'],
            ['e', id(5), '', 'reply'],
          ],
        ),
        event(7, channel: 'solo'),
        event(
          8,
          channel: 'dm',
          tags: [
            ['p', 'me'],
          ],
        ),
        event(
          9,
          tags: [
            ['p', 'me'],
          ],
        ),
        event(
          10,
          channel: 'solo',
          tags: [
            ['audience', 'agents'],
          ],
        ),
        event(
          11,
          channel: 'archived',
          tags: [
            ['p', 'me'],
          ],
        ),
      ];
      final items = await loadActivityHistory(
        query: reader(events),
        channels: [
          channel('team'),
          channel('solo', solo: true),
          channel('dm', dm: true),
          channel('archived', archived: true),
        ],
        viewer: 'me',
        followedRoots: {id(3), id(14)},
      );
      expect(
        items.map((e) => e.id),
        unorderedEquals([id(2), id(4), id(7), id(8), id(9), id(13), id(15)]),
      );
      expect(items.singleWhere((e) => e.id == id(8)).category, 'activity');
      expect(items.singleWhere((e) => e.id == id(9)).category, 'mention');
    },
  );
  test(
    'reactions require an owned target and honor edits and removals',
    () async {
      final events = [
        event(1, author: 'me', content: 'original'),
        event(
          2,
          kind: 7,
          tags: [
            ['e', id(1)],
          ],
          content: '👍',
        ),
        event(
          3,
          kind: 40003,
          author: 'me',
          tags: [
            ['e', id(1)],
          ],
          content: 'edited',
        ),
        event(4),
        event(
          5,
          kind: 7,
          tags: [
            ['p', 'me'],
            ['e', id(4)],
          ],
          content: '❤️',
        ),
        event(
          6,
          kind: 7,
          tags: [
            ['p', 'me'],
            ['e', id(1)],
          ],
          content: '🎉',
        ),
        event(
          7,
          kind: 5,
          tags: [
            ['e', id(6)],
          ],
        ),
        event(
          8,
          kind: 40003,
          author: 'forger',
          tags: [
            ['e', id(1)],
          ],
          content: 'forged',
        ),
      ];
      final items = await loadActivityHistory(
        query: reader(events),
        channels: [channel('team')],
        viewer: 'me',
        followedRoots: {},
      );
      expect(items, hasLength(1));
      expect(items.single.reactionTarget?.id, id(1));
      expect(items.single.content, 'edited');
      expect(items.single.reactionEmoji, '👍');
      expect(
        matchesInboxFilter(
          buildInboxItems(items).single,
          InboxFilter.reactions,
        ),
        isTrue,
      );
      events.add(
        event(
          9,
          author: 'me',
          kind: 5,
          tags: [
            ['e', id(1)],
          ],
        ),
      );
      expect(
        await loadActivityHistory(
          query: reader(events),
          channels: [channel('team')],
          viewer: 'me',
          followedRoots: {},
        ),
        isEmpty,
      );
    },
  );
  test('a reply cannot borrow ownership from another channel', () async {
    final items = await loadActivityHistory(
      query: reader([
        event(1, author: 'me', channel: 'a'),
        event(
          2,
          channel: 'b',
          tags: [
            ['e', id(1), '', 'root'],
            ['e', id(1), '', 'reply'],
          ],
        ),
      ]),
      channels: [channel('a'), channel('b')],
      viewer: 'me',
      followedRoots: {},
    );
    expect(items, isEmpty);
  });
  test(
    'cold aggregate pages retain quiet DMs, Solo and followed replies',
    () async {
      final data = [
        event(1, channel: 'quiet-dm'),
        event(2, channel: 'quiet-solo'),
        event(3, channel: 'quiet-team'),
        event(
          4,
          channel: 'quiet-team',
          tags: [
            ['e', id(3), '', 'reply'],
          ],
        ),
        for (var n = 10; n < 1010; n++) ...[
          event(n, channel: 'busy-dm'),
          event(n + 2000, channel: 'busy-team'),
        ],
      ];
      final calls = <List<NostrFilter>>[];
      final items = await loadActivityHistory(
        query: (filters) async {
          calls.add(filters);
          return reader(data)(filters);
        },
        channels: [
          channel('quiet-dm', dm: true),
          channel('busy-dm', dm: true),
          channel('quiet-solo', solo: true),
          channel('quiet-team'),
          channel('busy-team'),
        ],
        viewer: 'me',
        followedRoots: {id(3)},
      );
      expect(items.map((e) => e.id), containsAll([id(1), id(2), id(4)]));
      final cold = calls
          .expand((f) => f)
          .where((f) => f.tags.containsKey('#h'));
      expect(cold, hasLength(4));
      expect(
        cold
            .where((f) => f.until != null)
            .every((f) => !f.tags['#h']!.any((id) => id.startsWith('busy'))),
        isTrue,
      );
    },
  );
  test(
    'cold history reaches quiet channels beyond 32 saturated pages',
    () async {
      final data = [
        event(1, channel: 'quiet'),
        for (var c = 0; c < 34; c++)
          for (var n = 0; n < 500; n++)
            event(10 + c * 500 + n, channel: 'busy-$c'),
      ];
      final items = await loadActivityHistory(
        query: reader(data),
        channels: [
          channel('quiet', dm: true),
          for (var c = 0; c < 34; c++) channel('busy-$c', dm: true),
        ],
        viewer: 'me',
        followedRoots: {},
      );
      expect(items.map((e) => e.id), contains(id(1)));
    },
  );
  test(
    'aggregate participation pages before caching absent interests',
    () async {
      final items = await loadActivityHistory(
        query: reader([
          event(
            1,
            author: 'me',
            tags: [
              ['e', id(2), '', 'reply'],
            ],
          ),
          event(2),
          event(3),
          for (var n = 100; n < 601; n++)
            event(
              n,
              author: 'me',
              tags: [
                ['e', id(3), '', 'reply'],
              ],
            ),
          event(
            700,
            tags: [
              ['e', id(2), '', 'reply'],
            ],
          ),
        ]),
        channels: [channel('team')],
        viewer: 'me',
        followedRoots: {},
      );
      expect(items.map((e) => e.id), contains(id(700)));
    },
  );
  test(
    'cold membership reads batch 300 channels under the relay cap',
    () async {
      final calls = <List<NostrFilter>>[];
      await loadActivityHistory(
        query: (filters) async {
          calls.add(filters);
          return [];
        },
        channels: [for (var n = 0; n < 300; n++) channel('channel-$n')],
        viewer: 'me',
        followedRoots: {},
      );
      final scoped = calls
          .expand((f) => f)
          .where((f) => f.tags.containsKey('#h'));
      expect(scoped, hasLength(3));
      expect(calls.length, lessThanOrEqualTo(4));
      expect(scoped.expand((f) => f.tags['#h']!), hasLength(300));
      for (final call in calls) {
        expect(
          call.fold<int>(0, (n, f) => n + (f.tags['#h']?.length ?? 0)),
          lessThanOrEqualTo(kMaxExplicitChannelValues),
        );
      }
    },
  );
  test(
    'incremental catch-up retains older conversations and fences failed or retired reads',
    () async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final data = [
        event(now - 10, channel: 'dm'),
        event(now - 9, channel: 'solo'),
      ];
      final cache = ActivityHistoryCache();
      final filters = <NostrFilter>[];
      Future<List<NostrEvent>> query(List<NostrFilter> next) async {
        filters.addAll(next);
        return reader(data)(next);
      }

      Future<List<dynamic>> load({bool current = true}) => loadActivityHistory(
        query: query,
        channels: [channel('dm', dm: true), channel('solo', solo: true)],
        viewer: 'me',
        followedRoots: {},
        cache: cache,
        isCurrent: () => current,
      );
      expect(await load(), hasLength(2));
      filters.clear();
      data.add(event(now, channel: 'dm'));
      expect(await load(), hasLength(3));
      final catchUp = filters
          .where((f) => f.tags.containsKey('#h') && f.authors == null)
          .toList();
      expect(catchUp, hasLength(1));
      expect(catchUp.single.tags['#h'], unorderedEquals(['dm', 'solo']));
      expect(catchUp.single.since, isNotNull);
      final before = cache.events.keys.toSet();
      data.add(event(now + 1, channel: 'solo'));
      await load(current: false);
      expect(cache.events.keys.toSet(), before);
      await expectLater(
        loadActivityHistory(
          query: (_) async => throw StateError('offline'),
          channels: [channel('dm', dm: true)],
          viewer: 'me',
          followedRoots: {},
          cache: cache,
        ),
        throwsStateError,
      );
      expect(cache.events.keys.toSet(), before);
    },
  );
}
