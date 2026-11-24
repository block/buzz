part of '../channels_page_test.dart';

void _readSeedJourney(
  Widget Function({required List<Override> overrides, SharedPreferences? prefs})
  buildTestable,
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
          prefs: prefs,
          overrides: [
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

  testWidgets('first landing baselines only after read state loads', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keys.nsec,
      pubkey: keys.public,
    )!;
    // A mark on this device and one from another device both predate the
    // newest message, so both channels are really unread.
    await prefs.setString(
      localReadStateKey(keys.public, 'https://seed.example'),
      jsonEncode({
        'channel-0': DateTime.fromMillisecondsSinceEpoch(
          (now - 10) * 1000,
          isUtc: true,
        ).toIso8601String(),
      }),
    );
    final peerMark = NostrEvent(
      id: 'peer-read-state',
      pubkey: keys.public,
      createdAt: now - 5,
      kind: EventKind.readState,
      tags: [
        ['d', '${readStateDTagPrefix}peer'],
        ['t', 'read-state'],
      ],
      content: crypto.encrypt(
        jsonEncode(
          ReadStateBlob(
            clientId: 'peer',
            contexts: {'channel-1': now - 5},
          ).toJson(),
        ),
      ),
      sig: '',
    );
    final scope =
        '${keys.public}:${Uri.encodeComponent('https://seed.example')}';
    Future<ProviderContainer> mount(_ReadSeedSession session) async {
      await tester.pumpWidget(
        buildTestable(
          prefs: prefs,
          overrides: [
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

    FontWeight? weight(String name) =>
        tester.widget<Text>(find.text(name)).style?.fontWeight;

    // The first load fails: nothing is baselined and the unread channels
    // stay unread, across a restart too.
    final offline = TimeoutException('offline');
    var session = _ReadSeedSession(keys.public, now)..snapshotError = offline;
    var container = await mount(session);
    expect(container.read(readStateProvider).contexts, {'channel-0': now - 10});
    expect(weight('channel-0'), FontWeight.w600);
    expect(weight('channel-1'), FontWeight.w600);
    // The landing is captured as it was, to apply once read state loads.
    expect(ReadStateStorage(prefs).readJournal(scope)!['seedBaseline'], {
      for (var i = 0; i < 400; i++) 'channel-$i': now,
    });
    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();
    session = _ReadSeedSession(keys.public, now)..snapshotError = offline;
    container = await mount(session);
    expect(container.read(readStateProvider).contexts, {'channel-0': now - 10});
    expect(weight('channel-1'), FontWeight.w600);

    // The retry loads: only channels with no mark anywhere are baselined.
    session
      ..snapshotError = null
      ..snapshot = [peerMark];
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
    final reads = container.read(readStateProvider).contexts;
    expect(reads, {
      'channel-0': now - 10,
      'channel-1': now - 5,
      for (var i = 2; i < 400; i++) 'channel-$i': now,
    });
    expect(weight('channel-0'), FontWeight.w600);
    expect(weight('channel-1'), FontWeight.w600);
    expect(weight('channel-10'), FontWeight.w400);
    final saved = ReadStateStorage(prefs).readJournal(scope)!;
    expect(saved.containsKey('seedBaseline'), isFalse);
    expect(saved.containsKey('seedPending'), isFalse);
    expect(saved['observed'], reads);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  Future<ProviderContainer> mountReadSeed(
    WidgetTester tester,
    SharedPreferences prefs,
    nostr.Keys keys,
    _ReadSeedSession session,
  ) async {
    await tester.pumpWidget(
      buildTestable(
        prefs: prefs,
        overrides: [
          relayConfigProvider.overrideWith(() => _ReadSeedConfig(keys.nsec)),
          relaySessionProvider.overrideWith(() => session),
          activeCommunityProvider.overrideWith((ref) async => null),
        ],
      ),
    );
    await tester.pumpAndSettle();
    return ProviderScope.containerOf(tester.element(find.byType(ChannelsPage)));
  }

  FontWeight? weightOf(WidgetTester tester, String name) =>
      tester.widget<Text>(find.text(name)).style?.fontWeight;

  for (final restart in [false, true]) {
    testWidgets(
      'a post after a failed first landing stays unread (restart: $restart)',
      (tester) async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final keys = nostr.Keys.generate();
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final landing = now - 100;
        var session = _ReadSeedSession(keys.public, landing)
          ..snapshotError = TimeoutException('offline');
        var container = await mountReadSeed(tester, prefs, keys, session);
        expect(weightOf(tester, 'channel-10'), FontWeight.w600);

        // channel-10 gets a new post, then a snapshot with no marks loads.
        if (restart) {
          await tester.pumpWidget(const SizedBox());
          await tester.pumpAndSettle();
          session = _ReadSeedSession(keys.public, landing)..later = now;
          container = await mountReadSeed(tester, prefs, keys, session);
        } else {
          session.later = now;
          await container.read(channelsProvider.notifier).refresh();
          await tester.pumpAndSettle();
          session.snapshotError = null;
          await tester.pump(const Duration(seconds: 6));
          await tester.pumpAndSettle();
        }
        final reads = container.read(readStateProvider);
        expect(reads.initialSeedComplete, isTrue);
        expect(reads.contexts['channel-10'], landing);
        expect(weightOf(tester, 'channel-10'), FontWeight.w600);
        expect(weightOf(tester, 'channel-0'), FontWeight.w400);
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox());
      },
    );
  }

  testWidgets('a channel joined after a failed first landing stays unread', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final session = _ReadSeedSession(keys.public, now)
      ..snapshotError = TimeoutException('offline');
    final container = await mountReadSeed(tester, prefs, keys, session);

    session.joined = true;
    await container.read(channelsProvider.notifier).refresh();
    await tester.pumpAndSettle();
    session.snapshotError = null;
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
    final reads = container.read(readStateProvider);
    expect(reads.initialSeedComplete, isTrue);
    expect(reads.contexts.containsKey('channel-00-joined'), isFalse);
    expect(weightOf(tester, 'channel-00-joined'), FontWeight.w600);
    expect(weightOf(tester, 'channel-0'), FontWeight.w400);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('an empty failed first landing keeps later channels unread', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final scope =
        '${keys.public}:${Uri.encodeComponent('https://seed.example')}';
    final first = _ReadSeedSession(keys.public, now)
      ..available = false
      ..snapshotError = TimeoutException('offline');
    await mountReadSeed(tester, prefs, keys, first);
    expect(ReadStateStorage(prefs).readJournal(scope)!['seedBaseline'], {});

    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();
    final container = await mountReadSeed(
      tester,
      prefs,
      keys,
      _ReadSeedSession(keys.public, now),
    );
    final reads = container.read(readStateProvider);
    expect(reads.initialSeedComplete, isTrue);
    expect(reads.contexts, isEmpty);
    expect(weightOf(tester, 'channel-0'), FontWeight.w600);
    final saved = ReadStateStorage(prefs).readJournal(scope)!;
    expect(saved.containsKey('seedBaseline'), isFalse);
    expect(saved.containsKey('seedPending'), isFalse);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a relay without the snapshot extension baselines the landing', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final session = _ReadSeedSession(keys.public, now)
      ..snapshotError = const ReadStateSnapshotUnsupported();
    final container = await mountReadSeed(tester, prefs, keys, session);
    final reads = container.read(readStateProvider);
    expect(reads.loadPending, isFalse);
    expect(reads.initialSeedComplete, isTrue);
    expect(reads.contexts, {for (var i = 0; i < 400; i++) 'channel-$i': now});
    expect(weightOf(tester, 'channel-0'), FontWeight.w400);
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
        prefs: prefs,
        overrides: [
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
  // A newer post in channel-10, at this time.
  int? later;
  // The user is also a member of channel-00-joined.
  bool joined = false;
  Object? snapshotError;
  List<NostrEvent> snapshot = const [];
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  Iterable<String> get channels => [
    for (var i = 0; i < 400; i++) 'channel-$i',
    if (joined) 'channel-00-joined',
  ];

  NostrEvent event(int kind, String channel, List<List<String>> tags) {
    final post = kind == 9 && channel == 'channel-10' ? later : null;
    return NostrEvent(
      id: post == null ? '$kind-$channel' : '$kind-$channel-$post',
      pubkey: kind == 9 ? 'other-member' : pubkey,
      createdAt: post ?? now,
      kind: kind,
      tags: tags,
      content: '',
      sig: '',
    );
  }

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
        for (final id in channels)
          event(39002, id, [
            ['d', id],
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
  }) async {
    if (snapshotError case final error?) throw error;
    return snapshot;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => available && filter.kinds.contains(39000)
      ? [
          for (final id in channels)
            event(39000, id, [
              ['d', id],
              ['name', id],
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
    bool resumeSince = true,
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
    Future<bool> Function()? mayPublish,
  }) async {
    if (event.kind == EventKind.readState) readStatePublishes++;
    return event;
  }
}
