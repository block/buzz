part of '../channels_page_test.dart';

void _readSeedJourney(
  Widget Function({required List<Override> overrides}) buildTestable,
) {
  testWidgets('late unseen channels stay unread after journal reload', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final first = _ReadSeedSession(keys.public, now)..available = false;
    Future<ProviderContainer> mount(_ReadSeedSession session) async {
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
      return ProviderScope.containerOf(
        tester.element(find.byType(ChannelsPage)),
      );
    }

    final container = await mount(first);
    expect(container.read(readStateProvider).contexts, isEmpty);
    expect(find.text('channel-0'), findsNothing);
    first.available = true;
    await container.read(channelsProvider.notifier).refresh();
    await tester.pumpAndSettle();
    expect(container.read(readStateProvider).contexts, isEmpty);
    expect(
      tester.widget<Text>(find.text('channel-0')).style?.fontWeight,
      FontWeight.w600,
    );
    await tester.pump(const Duration(seconds: 6));
    final scope =
        '${keys.public}:${Uri.encodeComponent('https://seed.example')}';
    final before = ReadStateStorage(prefs).readJournal(scope)!;
    expect(before['observed'], isEmpty);
    expect(first.readStatePublishes, 0);
    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();
    final second = _ReadSeedSession(keys.public, now);
    final restarted = await mount(second);
    final reads = restarted.read(readStateProvider);
    expect(reads.contexts, isEmpty);
    expect(
      tester.widget<Text>(find.text('channel-0')).style?.fontWeight,
      FontWeight.w600,
    );
    final after = ReadStateStorage(prefs).readJournal(scope)!;
    expect(after['observed'], isEmpty);
    expect(after['local'], isEmpty);
    expect(after['published'], isEmpty);
    await tester.pump(const Duration(seconds: 6));
    expect(second.readStatePublishes, 0);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('cold community seeds read state with one saved snapshot', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final store = _ReadSeedStore();
    SharedPreferencesStorePlatform.instance = store;
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final session = _ReadSeedSession(keys.public, now);
    final readKey = localReadStateKey(keys.public, 'https://seed.example');
    // Preserve a real pre-existing unread frontier while seeding new channels.
    await prefs.setString(
      readKey,
      jsonEncode({
        'channel-0': DateTime.fromMillisecondsSinceEpoch(
          (now - 10) * 1000,
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
      'channel-0': now - 10,
      for (var i = 1; i < 400; i++) 'channel-$i': now,
    });
    // Ignore hydration saves of the existing mark; landing saves all seeds together.
    expect(store.nonemptySnapshots.where((count) => count > 1), [400]);
    expect(
      tester.widget<Text>(find.text('channel-0')).style?.fontWeight,
      FontWeight.w600,
    );
    expect(
      tester.widget<Text>(find.text('channel-1')).style?.fontWeight,
      FontWeight.w400,
    );
    final saved = ReadStateStorage(prefs).readJournal(
      '${keys.public}:${Uri.encodeComponent('https://seed.example')}',
    )!;
    expect(saved['observed'], reads.contexts);
    expect(saved['local'], isEmpty);
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
    if (key.contains('buzz.nip-rs.journal.v1:')) {
      final contexts = (jsonDecode(value as String) as Map)['observed'] as Map;
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
  _ReadSeedSession(this.pubkey, this.now);
  final int now;
  final String pubkey;
  int readStatePublishes = 0;
  bool available = true;
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  NostrEvent event(int kind, String channel, List<List<String>> tags) =>
      NostrEvent(
        id: '$kind-$channel',
        pubkey: kind == 9 ? 'other-member' : pubkey,
        createdAt: now,
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
    if (!available) return [];
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
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => available && filter.kinds.contains(39000)
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
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
  }) => subscribe(filter, onEvent, onClosed: onClosed);

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
