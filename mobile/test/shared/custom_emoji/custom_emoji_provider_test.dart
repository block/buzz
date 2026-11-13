import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test('a reconnect reads only sets written since the previous read', () async {
    final relay = _EmojiRelay()
      ..sets['alice'] = _set('alice', 1000, {'a': 'https://e/a.png'});
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relay),
        relayClientProvider.overrideWithValue(
          RelayClient(baseUrl: 'http://localhost:3000'),
        ),
      ],
    );
    addTearDown(container.dispose);
    container.listen(customEmojiPaletteProvider, (_, _) {});

    Future<List<String>> palette() async => [
      for (final emoji in await container.read(
        customEmojiPaletteProvider.future,
      ))
        emoji.shortcode,
    ];
    Future<void> reconnect() async {
      relay.setStatus(SessionStatus.disconnected);
      await container.read(customEmojiPaletteProvider.future);
      relay.setStatus(SessionStatus.connected);
    }

    expect(await palette(), ['a']);
    expect(relay.since, [null]);

    // Nothing new: the reconnect reads a short window, and the known sets stay.
    await reconnect();
    expect(await palette(), ['a']);
    expect(relay.since, hasLength(2));
    expect(relay.since.last, isNotNull);

    // A new member's set and a replaced set both arrive in that window.
    relay.sets['bob'] = _set('bob', relay.since.last! + 10, {
      'b': 'https://e/b.png',
    });
    relay.sets['alice'] = _set('alice', relay.since.last! + 20, {
      'c': 'https://e/c.png',
    });
    await reconnect();
    expect(await palette(), ['b', 'c']);

    // Nothing new again: the reconnect publishes the identical list.
    final unchanged = await container.read(customEmojiPaletteProvider.future);
    await reconnect();
    expect(
      await container.read(customEmojiPaletteProvider.future),
      same(unchanged),
    );

    // On a same-second tie the relay keeps the lower id, and so does the
    // palette.
    final second = relay.sets['alice']!.createdAt;
    relay.sets['alice'] = _set('alice', second, {
      'd': 'https://e/d.png',
    }, id: '0');
    await reconnect();
    expect(await palette(), ['b', 'd']);
    relay.sets['alice'] = _set('alice', second, {
      'e': 'https://e/e.png',
    }, id: 'z');
    await reconnect();
    expect(await palette(), ['b', 'd']);
  });

  test(
    'a day after the last full read, a reconnect drops deleted sets',
    () async {
      var now = 1000000;
      final relay = _EmojiRelay()
        ..sets['alice'] = _set('alice', 1000, {'a': 'https://e/a.png'})
        ..sets['bob'] = _set('bob', 1000, {'b': 'https://e/b.png'});
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => relay),
          relayClientProvider.overrideWithValue(
            RelayClient(baseUrl: 'http://localhost:3000'),
          ),
          customEmojiPaletteProvider.overrideWith(
            () => CustomEmojiPaletteNotifier(now: () => now),
          ),
        ],
      );
      addTearDown(container.dispose);
      container.listen(customEmojiPaletteProvider, (_, _) {});

      Future<List<CustomEmoji>> read() =>
          container.read(customEmojiPaletteProvider.future);
      Future<List<String>> palette() async => [
        for (final emoji in await read()) emoji.shortcode,
      ];
      Future<void> reconnectAfter(int seconds) async {
        now += seconds;
        relay.setStatus(SessionStatus.disconnected);
        await read();
        relay.setStatus(SessionStatus.connected);
        await read();
      }

      expect(await palette(), ['a', 'b']);

      // Bob deletes his set; a `since` read can't tell.
      relay.sets.remove('bob');
      await reconnectAfter(60);
      expect(relay.since.last, isNotNull);
      expect(await palette(), ['a', 'b']);

      // A day on, the reconnect reads every set and drops Bob's.
      await reconnectAfter(CustomEmojiPaletteNotifier.fullReadEverySeconds);
      expect(relay.since.last, isNull);
      expect(await palette(), ['a']);

      // An unchanged full read keeps the same list, and the next read is
      // incremental again.
      final unchanged = await read();
      await reconnectAfter(CustomEmojiPaletteNotifier.fullReadEverySeconds);
      expect(relay.since.last, isNull);
      expect(await read(), same(unchanged));
      await reconnectAfter(60);
      expect(relay.since.last, isNotNull);
      expect(await read(), same(unchanged));
      expect(relay.since, hasLength(5));
    },
  );

  test('a capped full read keeps every cached set', () async {
    var now = 1000000;
    const limit = CustomEmojiPaletteNotifier.readLimit;
    final relay = _EmojiRelay();
    for (var i = 0; i < limit; i++) {
      relay.sets['u$i'] = _set('u$i', 1000 + i, {'e$i': 'https://e/$i.png'});
    }
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relay),
        relayClientProvider.overrideWithValue(
          RelayClient(baseUrl: 'http://localhost:3000'),
        ),
        customEmojiPaletteProvider.overrideWith(
          () => CustomEmojiPaletteNotifier(now: () => now),
        ),
      ],
    );
    addTearDown(container.dispose);
    container.listen(customEmojiPaletteProvider, (_, _) {});

    Future<List<CustomEmoji>> read() =>
        container.read(customEmojiPaletteProvider.future);
    Future<void> reconnectAfter(int seconds) async {
      now += seconds;
      relay.setStatus(SessionStatus.disconnected);
      await read();
      relay.setStatus(SessionStatus.connected);
      await read();
    }

    expect(await read(), hasLength(limit));

    // One more member's set arrives incrementally, so the relay now holds
    // more live sets than a read returns.
    relay.sets['new'] = _set('new', now, {'new': 'https://e/new.png'});
    await reconnectAfter(60);
    expect(await read(), hasLength(limit + 1));

    // The day's full read is capped and leaves the oldest live set out; the
    // palette keeps it, and the next read waits a day rather than going full.
    await reconnectAfter(CustomEmojiPaletteNotifier.fullReadEverySeconds);
    expect(relay.since.last, isNull);
    expect(await read(), hasLength(limit + 1));
    await reconnectAfter(60);
    expect(relay.since.last, isNotNull);
  });
}

NostrEvent _set(
  String pubkey,
  int createdAt,
  Map<String, String> emoji, {
  String? id,
}) => NostrEvent(
  id: id ?? '$pubkey$createdAt',
  pubkey: pubkey,
  createdAt: createdAt,
  kind: kindEmojiSet,
  tags: [
    ['d', customEmojiSetDTag],
    for (final MapEntry(:key, :value) in emoji.entries) ['emoji', key, value],
  ],
  content: '',
  sig: '',
);

class _EmojiRelay extends RelaySessionNotifier {
  final Map<String, NostrEvent> sets = {};
  final List<int?> since = [];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void setStatus(SessionStatus status) => state = SessionState(status: status);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    expect(filter.kinds, [kindEmojiSet]);
    since.add(filter.since);
    final matching = [
      for (final event in sets.values)
        if (event.createdAt >= (filter.since ?? 0)) event,
    ]..sort((a, b) => b.createdAt.compareTo(a.createdAt));
    return matching.take(filter.limit).toList();
  }
}
