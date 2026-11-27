part of '../channels_page_test.dart';

void _filterFeedbackTests(
  Widget Function({
    required List<Override> overrides,
    bool realHome,
    SharedPreferences? prefs,
  })
  buildTestable,
) {
  testWidgets(
    'deleting an unfiltered section persists offline without filter publication',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys.generate();
      const relayUrl = 'https://offline.example';
      final storage = ChannelSectionsStorage(prefs);
      storage.write(
        keys.public,
        relayUrl,
        const ChannelSectionStore(
          sections: [
            ChannelSection(id: 'offline', name: 'Offline section', order: 0),
          ],
        ),
      );
      var publications = 0;
      final manager = _filterTestManager(onPublish: () => publications++);
      await tester.pumpWidget(
        buildTestable(
          prefs: prefs,
          overrides: [
            channelsProvider.overrideWith(
              () => _FakeNotifier([
                Channel(
                  id: 'channel',
                  name: 'General',
                  channelType: 'stream',
                  visibility: 'open',
                  description: '',
                  memberCount: 1,
                  createdBy: 'pk',
                  createdAt: DateTime(2025),
                  isMember: true,
                ),
              ]),
            ),
            relayConfigProvider.overrideWith(
              () => _FilterFeedbackConfig(
                RelayConfig(baseUrl: relayUrl, nsec: keys.nsec),
              ),
            ),
            relaySessionProvider.overrideWith(
              () => _ReconnectingRelaySession(
                initialStatus: SessionStatus.disconnected,
              ),
            ),
            channelFilterProvider.overrideWithValue(manager),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(manager.ready, isFalse);
      expect(find.text('Offline section'), findsOneWidget);
      await tester.tap(find.byKey(const Key('section-menu-offline')));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Delete section'));
      await tester.tap(find.text('Delete section'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();
      expect(find.text('Offline section'), findsNothing);
      expect(storage.read(keys.public, relayUrl).sections, isEmpty);
      expect(publications, 0);
      await tester.pumpWidget(const SizedBox());
      manager.dispose();
    },
  );

  testWidgets('Unreads agrees with highlights in the pre-seed frame', (
    tester,
  ) async {
    final channels = [
      for (final id in ['history', 'forced'])
        Channel(
          id: id,
          name: id,
          channelType: 'stream',
          visibility: 'open',
          description: '',
          memberCount: 1,
          createdBy: 'pk',
          createdAt: DateTime(2025),
          lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
            20000,
            isUtc: true,
          ),
          isMember: true,
        ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {},
        version: 0,
        forcedUnreadContexts: {'forced': 'forced'},
      ),
    );
    final manager = _filterTestManager(
      cached: ChannelFilterStore.empty().edit(
        'channels',
        ChannelFilterMode.unreads,
      ),
    );
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(
            () => _FakeNotifier(
              channels,
              observedEventsByChannel: {
                'history': [_observed(id: 'historical', createdAt: 20)],
              },
            ),
          ),
          readStateProvider.overrideWith(() => readState),
          channelFilterProvider.overrideWithValue(manager),
        ],
      ),
    );
    // The channel future resolves before this frame. Seeding runs at its end,
    // but the rendered frame still reflects timestamp-less read state.
    await tester.pump();
    expect(readState.seededContexts, {'history': 20, 'forced': 20});
    expect(find.text('history'), findsNothing);
    expect(find.text('forced'), findsOneWidget);
    expect(
      tester.widget<Text>(find.text('forced')).style?.fontWeight,
      FontWeight.w600,
    );
    await tester.pumpAndSettle();
    expect(find.text('history'), findsNothing);
    expect(find.text('See all'), findsOneWidget);
    expect(find.text('forced'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    manager.dispose();
  });
  testWidgets(
    'section options survive same-scope community writes and reconnects',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys.generate();
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final community = Community.create(
        name: 'Filter community',
        relayUrl: 'https://filters.example',
        nsec: keys.nsec,
        pubkey: keys.public,
      );
      final other = Community.create(
        name: 'Other community',
        relayUrl: 'https://other.example',
        nsec: keys.nsec,
        pubkey: keys.public,
      );
      await storage.save(community);
      await storage.save(other);
      await storage.saveActiveId(community.id);
      final relay = _FilterJourneyRelay();
      await tester.pumpWidget(
        buildTestable(
          prefs: prefs,
          overrides: [
            channelsProvider.overrideWith(
              () => _FakeNotifier([
                Channel(
                  id: 'channel',
                  name: 'General',
                  channelType: 'stream',
                  visibility: 'open',
                  description: '',
                  memberCount: 1,
                  createdBy: 'pk',
                  createdAt: DateTime(2025),
                  isMember: true,
                ),
              ]),
            ),
            readStateProvider.overrideWith(
              () => _FakeReadStateNotifier(
                const ReadStateState(
                  isReady: true,
                  pubkey: 'pk',
                  contexts: {},
                  version: 0,
                ),
              ),
            ),
            communityStorageProvider.overrideWithValue(storage),
            communitySnapshotWriterProvider.overrideWithValue((_) async {}),
            relaySessionProvider.overrideWith(() => relay),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ChannelsPage)),
      );
      final communities = container.read(communityListProvider.notifier);
      final manager = container.read(channelFilterProvider)!;
      expect(manager.ready, isTrue);
      const reopen = 'Preferences changed. Reopen section options to continue.';
      IconButton applyButton() => tester.widget<IconButton>(
        find.byWidgetPredicate(
          (widget) => widget is IconButton && widget.tooltip == 'Apply changes',
        ),
      );

      // A push lease write lands while a filter save awaits confirmation: the
      // save completes and the co-staged sort still applies.
      await tester.tap(find.byTooltip('Team options'));
      await tester.pumpAndSettle();
      await _chooseSectionOption(tester, 'Sort', 'Recent');
      await _chooseSectionOption(tester, 'View', 'Unreads');
      await tester.tap(find.byTooltip('Apply changes'));
      await tester.pumpAndSettle();
      expect(relay.filterPublishes, 1);
      expect(find.byTooltip('Saving…'), findsOneWidget);
      await communities.reservePushLeaseGeneration(community.id);
      await tester.pumpAndSettle();
      expect(container.read(channelFilterProvider), same(manager));
      expect(find.text(reopen), findsNothing);
      relay.releaseHeld();
      await tester.pumpAndSettle();
      expect(find.byTooltip('Apply changes'), findsNothing);
      expect(manager.store.modeFor('channels'), ChannelFilterMode.unreads);
      expect(
        container.read(channelSortProvider).sortModeFor('channels'),
        ChannelSortMode.recent,
      );

      // A resume read starts on the old socket, the replacement connects
      // while it is in flight, then the old read fails. The sheet and manager
      // survive, a sort-only Apply goes through after a reverted filter edit,
      // and the replacement's queued refresh restores write access.
      await tester.tap(find.byTooltip('Team options'));
      await tester.pumpAndSettle();
      await _chooseSectionOption(tester, 'View', 'All chats');
      await _chooseSectionOption(tester, 'View', 'Unreads');
      await _chooseSectionOption(tester, 'Sort', 'A–Z');
      relay.holdNextFilterRead = true;
      unawaited(manager.refresh());
      relay.setReconnecting();
      await tester.pumpAndSettle();
      relay.connect();
      await tester.pumpAndSettle();
      expect(container.read(channelFilterProvider), same(manager));
      expect(manager.busy, isTrue);
      expect(find.text(reopen), findsNothing);
      expect(applyButton().onPressed, isNotNull);
      await tester.tap(find.byTooltip('Apply changes'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Apply changes'), findsNothing);
      expect(
        container.read(channelSortProvider).sortModeFor('channels'),
        ChannelSortMode.alpha,
      );
      relay.releaseHeld(fail: true);
      await tester.pumpAndSettle();
      expect(manager.busy, isFalse);
      expect(manager.ready, isTrue);
      expect(manager.store.modeFor('channels'), ChannelFilterMode.unreads);

      // A failed load leaves a staged filter edit recoverable from the sheet.
      await tester.tap(find.byTooltip('Team options'));
      await tester.pumpAndSettle();
      await _chooseSectionOption(tester, 'View', 'All chats');
      relay.setReconnecting();
      await tester.pumpAndSettle();
      relay.holdNextFilterRead = true;
      relay.connect();
      await tester.pumpAndSettle();
      relay.releaseHeld(fail: true);
      await tester.pumpAndSettle();
      expect(manager.ready, isFalse);
      await tester.tap(find.byTooltip('Apply changes'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Retry loading preferences'));
      await tester.tap(find.text('Retry loading preferences'));
      await tester.pumpAndSettle();
      expect(manager.ready, isTrue);
      await tester.tap(find.byTooltip('Apply changes'));
      await tester.pumpAndSettle();
      relay.releaseHeld();
      await tester.pumpAndSettle();
      expect(find.byTooltip('Apply changes'), findsNothing);
      expect(manager.store.modeFor('channels'), ChannelFilterMode.all);

      // Switching community still abandons the sheet.
      await tester.tap(find.byTooltip('Team options'));
      await tester.pumpAndSettle();
      await communities.switchCommunity(other.id);
      await tester.pumpAndSettle();
      expect(find.text(reopen), findsOneWidget);
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
    },
  );
}

class _FilterFeedbackConfig extends RelayConfigNotifier {
  _FilterFeedbackConfig(this.config);
  final RelayConfig config;
  @override
  RelayConfig build() => config;
}

/// Records filter publications and can hold one filter read open, so a test
/// can act while a save awaits confirmation or a refresh is in flight.
class _FilterJourneyRelay extends _ReconnectingRelaySession {
  _FilterJourneyRelay() : super(initialStatus: SessionStatus.connected);

  var _filterHistory = <NostrEvent>[];
  var filterPublishes = 0;
  var holdNextFilterRead = false;
  Completer<void>? _held;

  void releaseHeld({bool fail = false}) {
    final held = _held!;
    _held = null;
    fail
        ? held.completeError(TimeoutException('stale socket'))
        : held.complete();
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.tags['#d']?.contains('channel-filter') != true) return [];
    if (holdNextFilterRead) {
      holdNextFilterRead = false;
      _held = Completer<void>();
      await _held!.future;
    }
    return _filterHistory;
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
    Future<bool> Function()? mayPublish,
  }) async {
    if (event.getTagValue('d') == 'channel-filter') {
      filterPublishes++;
      _filterHistory = [event];
      // Hold the confirming readback that follows a filter publication.
      holdNextFilterRead = true;
    }
    return event;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) async => [];
}
