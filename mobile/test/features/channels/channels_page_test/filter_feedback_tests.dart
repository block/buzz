part of '../channels_page_test.dart';

void _filterFeedbackTests(
  Widget Function({required List<Override> overrides}) buildTestable,
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
            savedPrefsProvider.overrideWithValue(prefs),
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
      FontWeight.w700,
    );
    await tester.pumpAndSettle();
    expect(find.text('history'), findsNothing);
    await tester.tap(find.text('Show hidden channels · 1'));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Text>(find.text('history')).style?.fontWeight,
      FontWeight.w400,
    );
    await tester.pumpWidget(const SizedBox());
    manager.dispose();
  });
}

class _FilterFeedbackConfig extends RelayConfigNotifier {
  _FilterFeedbackConfig(this.config);
  final RelayConfig config;
  @override
  RelayConfig build() => config;
}
