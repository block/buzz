part of '../channels_page_test.dart';

void _readSeedJourney(
  Widget Function({required List<Override> overrides}) buildTestable,
) {
  testWidgets('cold community seeds read state with one saved snapshot', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final store = _ReadSeedStore();
    SharedPreferencesStorePlatform.instance = store;
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final session = _ReadSeedSession(keys.public);
    final readKey = localReadStateKey(keys.public, 'https://seed.example');
    // Preserve a real pre-existing unread frontier while seeding new channels.
    await prefs.setString(
      readKey,
      jsonEncode({
        'channel-0': DateTime.fromMillisecondsSinceEpoch(
          10000,
          isUtc: true,
        ).toIso8601String(),
      }),
    );
    store.nonemptySnapshots.clear();
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          relayConfigProvider.overrideWith(() => _ReadSeedConfig(keys.nsec)),
          relaySessionProvider.overrideWith(() => session),
          activeCommunityProvider.overrideWith((ref) async => null),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ChannelsPage)),
    );
    final reads = container.read(readStateProvider);
    expect(reads.contexts, {
      'channel-0': 10,
      for (var i = 1; i < 400; i++) 'channel-$i': 20,
    });
    // Ignore hydration saves of the existing mark; landing saves all seeds together.
    expect(store.nonemptySnapshots.where((count) => count > 1), [400]);
    expect(
      tester.widget<Text>(find.text('channel-0')).style?.fontWeight,
      FontWeight.w700,
    );
    expect(
      tester.widget<Text>(find.text('channel-1')).style?.fontWeight,
      FontWeight.w400,
    );
    final saved = ReadStateStorage(
      prefs,
    ).read(keys.public, 'https://seed.example');
    expect(saved.contexts, reads.contexts);
    expect(saved.publishableContextIds, isEmpty);
    await tester.pump(const Duration(seconds: 6));
    expect(session.readStatePublishes, 0);
    // A refresh/rebuild does not re-seed or save unchanged marks.
    await container.read(channelsProvider.notifier).refresh();
    await tester.pumpAndSettle();
    expect(store.nonemptySnapshots.where((count) => count > 1), [400]);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}

class _ReadSeedStore extends InMemorySharedPreferencesStore {
  _ReadSeedStore() : super.empty();
  final nonemptySnapshots = <int>[];

  @override
  Future<bool> setValue(String valueType, String key, Object value) async {
    if (key.contains('buzz.channel-read-state.v2:')) {
      final contexts = jsonDecode(value as String) as Map;
      if (contexts.isNotEmpty) nonemptySnapshots.add(contexts.length);
    }
    return super.setValue(valueType, key, value);
  }
}

class _ReadSeedConfig extends RelayConfigNotifier {
  _ReadSeedConfig(this.nsec);
  final String nsec;
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://seed.example', nsec: nsec);
}

class _ReadSeedSession extends RelaySessionNotifier {
  _ReadSeedSession(this.pubkey);
  final String pubkey;
  int readStatePublishes = 0;
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  NostrEvent event(int kind, String channel, List<List<String>> tags) =>
      NostrEvent(
        id: '$kind-$channel',
        pubkey: kind == 9 ? 'other-member' : pubkey,
        createdAt: 20,
        kind: kind,
        tags: tags,
        content: '',
        sig: '',
      );

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final filter = filters.first;
    if (filter.kinds.contains(39002)) {
      if (filter.until != null) return [];
      return [
        for (var i = 0; i < 400; i++)
          event(39002, 'channel-$i', [
            ['d', 'channel-$i'],
            ['p', pubkey],
          ]),
      ];
    }
    return [
      for (final f in filters)
        if (f.tags['#h'] != null)
          for (final id in f.tags['#h']!)
            event(9, id, [
              ['h', id],
            ]),
    ];
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => filter.kinds.contains(39000)
      ? [
          for (var i = 0; i < 400; i++)
            event(39000, 'channel-$i', [
              ['d', 'channel-$i'],
              ['name', 'channel-$i'],
              ['t', 'stream'],
              ['public'],
            ]),
        ]
      : [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async => () {};

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (event.kind == EventKind.readState) readStatePublishes++;
    return event;
  }
}
