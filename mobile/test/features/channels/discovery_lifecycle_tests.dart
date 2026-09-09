part of 'compose_bar_test.dart';

void discoveryLifecycleTests() {
  testWidgets(
    'terminal CLOSED recovers; signed roster removal drops open picker',
    (tester) async {
      final viewer = nostr.Keys.generate();
      final owner = nostr.Keys.generate();
      final agent = nostr.Keys.generate();
      final relay = nostr.Keys.generate();
      NostrEvent roster(int time, bool includeViewer) => signed(
        relay,
        39002,
        '',
        time: time,
        tags: [
          ['d', 'channel-1'],
          if (includeViewer) ['p', viewer.public],
          ['p', agent.public, '', 'bot'],
        ],
      );
      final events = [
        roster(100, true),
        profile(agent, [authTag(owner, agent.public)]),
        signed(agent, 10100, {
          'name': 'Helper Bot',
          'channel_ids': ['channel-1'],
        }),
        signed(
          owner,
          30177,
          {'name': 'Helper Bot', 'parallelism': 1, 'respond_to': 'anyone'},
          tags: [
            ['d', agent.public],
          ],
        ),
      ];
      // PolicySession replaces queries only: live REQ/EOSE/CLOSED/event handling
      // and subscription removal/disposal are the production relay SDK.
      final session = PolicySession(events);
      await tester.pumpWidget(
        _buildComposeBar(
          discoverySession: session,
          currentPubkey: viewer.public,
          uploadService: _testUploadService(viewer.nsec),
          channels: [_makeCurrentChannel()],
          onSend:
              (
                content,
                mentions, {
                mediaTags = const <List<String>>[],
              }) async {},
        ),
      );
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@hel');
      await tester.pump();
      session.debugHandleMessage(['EOSE', 'l-1']);
      await tester.pump(const Duration(milliseconds: 150));
      await tester.pumpAndSettle();
      final c = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      expect(c.read(agentDirectoryProvider).requireValue, hasLength(1));
      expect(find.text('Helper Bot'), findsOneWidget);
      session.debugHandleMessage(['CLOSED', 'l-1', 'restricted: terminal']);
      await tester.pump(const Duration(milliseconds: 150));
      await tester.pump();
      expect(c.read(agentDirectoryProvider).hasError, isTrue);
      expect(find.text('Helper Bot'), findsNothing);
      await tester.pump(const Duration(milliseconds: 100));
      session.debugHandleMessage(['EOSE', 'l-2']);
      await tester.pump(const Duration(milliseconds: 150));
      await tester.pumpAndSettle();
      expect(find.text('Helper Bot'), findsOneWidget);
      final removed = roster(101, false);
      events.add(removed);
      session.debugHandleMessage(['EVENT', 'l-2', removed.toJson()]);
      session.debugFlushEventBuffer();
      await tester.pump(const Duration(milliseconds: 150));
      await tester.pumpAndSettle();
      expect(c.read(agentDirectoryProvider).requireValue, isEmpty);
      expect(find.text('Helper Bot'), findsNothing);
      expect(find.byType(TextField), findsOneWidget); // same mounted editor
      await tester.pumpWidget(const SizedBox.shrink());
      session.debugDispose();
    },
  );
}
