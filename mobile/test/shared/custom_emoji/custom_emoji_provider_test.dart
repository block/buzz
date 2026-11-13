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
    return [
      for (final event in sets.values)
        if (event.createdAt >= (filter.since ?? 0)) event,
    ];
  }
}
