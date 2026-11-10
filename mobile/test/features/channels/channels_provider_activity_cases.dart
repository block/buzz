part of 'channels_provider_test.dart';

void _threadActivityCases() {
  testWidgets('open an unmentioned reply from Activity Threads', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(390, 844));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    const capturePath = String.fromEnvironment('THREAD_ACTIVITY_CAPTURE');
    final captureKey = GlobalKey();
    if (capturePath.isNotEmpty) {
      await tester.runAsync(() async {
        for (final font in {
          'Inter': 'assets/fonts/InterVariable.ttf',
          'BuzzTabler': 'assets/fonts/TablerIcons.ttf',
        }.entries) {
          await (FontLoader(
            font.key,
          )..addFont(rootBundle.load(font.value))).load();
        }
      });
    }
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final self = '1' * 64;
    final root = 'c' * 64;
    final reply = 'd' * 64;
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, self)],
      metadata: [_meta(id: _channelA, name: 'general')],
      recentMessages: [
        NostrEvent(
          id: root,
          pubkey: self,
          createdAt: now - 120,
          kind: 9,
          tags: const [
            ['h', _channelA],
          ],
          content: 'Project update',
          sig: '',
        ),
        NostrEvent(
          id: reply,
          pubkey: 'e' * 64,
          createdAt: now - 60,
          kind: 9,
          tags: [
            ['h', _channelA],
            ['e', root, '', 'root'],
            ['e', root, '', 'reply'],
          ],
          content: 'The thread reply is here',
          sig: '',
        ),
      ],
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        myPubkeyProvider.overrideWithValue(self),
        relaySessionProvider.overrideWith(() => session),
        appLifecycleProvider.overrideWith(_FakeAppLifecycleNotifier.new),
      ],
    );
    addTearDown(container.dispose);
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: RepaintBoundary(
          key: captureKey,
          child: MaterialApp(
            debugShowCheckedModeBanner: false,
            theme: AppTheme.light(),
            home: const ActivityPage(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    if (capturePath.isNotEmpty) {
      await tester.runAsync(() async {
        final boundary =
            captureKey.currentContext!.findRenderObject()!
                as RenderRepaintBoundary;
        final image = await boundary.toImage(pixelRatio: 2);
        final data = await image.toByteData(format: ui.ImageByteFormat.png);
        await File(capturePath).writeAsBytes(data!.buffer.asUint8List());
        image.dispose();
      });
    }
    expect(find.text('The thread reply is here'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('activity-type-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Threads').last);
    await tester.pumpAndSettle();
    expect(find.text('The thread reply is here'), findsOneWidget);
    await tester.tap(find.byKey(ValueKey('inbox-row-$reply')));
    await tester.pumpAndSettle();
    // The row carries the reply and its thread into the conversation route.
    final destination = tester.widget<ChannelDetailPage>(
      find.byType(ChannelDetailPage),
    );
    expect(destination.initialMessageId, reply);
    expect(destination.initialThreadRootId, root);
    await tester.pumpWidget(const SizedBox.shrink());
    container.dispose();
    await tester.pumpAndSettle();
  });

  test(
    'channel thread attention has an Activity destination without new reads',
    () async {
      final root = 'a' * 64;
      final otherRoot = 'b' * 64;
      NostrEvent reply(
        String id,
        int at, {
        String? inRoot,
        String author = 'alice',
        List<List<String>> extraTags = const [],
      }) => NostrEvent(
        id: id,
        pubkey: author,
        createdAt: at,
        kind: 9,
        tags: [
          ['h', _channelA],
          ['e', inRoot ?? root, '', 'root'],
          ['e', inRoot ?? root, '', 'reply'],
          ...extraTags,
        ],
        content: 'Reply $id',
        sig: '',
      );
      final cold = reply('cold', 20);
      final mentioned = reply(
        'mention',
        30,
        extraTags: [
          ['p', 'me'],
        ],
      );
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, 'me')],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [
          cold,
          mentioned,
          reply('unrelated', 40, inRoot: otherRoot),
          reply(
            'quiet',
            45,
            extraTags: [
              ['audience', 'agents'],
            ],
          ),
          NostrEvent(
            id: root,
            pubkey: 'me',
            createdAt: 10,
            kind: 9,
            tags: const [
              ['h', _channelA],
            ],
            content: 'My thread',
            sig: '',
          ),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      final channels = container.read(channelsProvider.notifier);
      await channels.waitForUnreadCatchUp();
      await container.read(activityProvider.future);
      final subscription = container.listen(inboxItemsProvider, (_, _) {});
      addTearDown(subscription.close);

      // The addressed feed still has only the mention. All/Threads also expose
      // the non-mentioned reply already used by the channel's attention dot.
      expect(
        container.read(activityProvider).requireValue.all.map((e) => e.id),
        ['mention'],
      );
      var row = container.read(inboxItemsProvider).single;
      expect(
        row.groupItems.map((e) => e.id),
        unorderedEquals(['cold', 'mention']),
      );
      expect(matchesInboxFilter(row, InboxFilter.thread), isTrue);
      expect(
        row.groupItems.where((e) => e.category == 'mention').single.id,
        'mention',
      );
      expect(row.deepLinkTarget((_) => false).id, 'cold');
      expect(row.deepLinkTarget((_) => false).threadHeadId, root);

      final reads = session.queryBatches.length;
      final subscriptions = session.totalSubscribeCount;
      session.emit(reply('live', 50));
      row = container.read(inboxItemsProvider).single;
      expect(row.id, 'live');
      expect(row.groupItems, hasLength(3));
      expect(session.queryBatches, hasLength(reads));
      expect(session.totalSubscribeCount, subscriptions);
      // The existing thread read marker clears these rows without reading the
      // whole channel. Ordinary channel catch-up does not read a thread.
      expect(
        isInboxItemDone(
          row,
          markerOf: (key) => key == 'activity:$_channelA' ? 50 : null,
          localUnreadOverrides: {},
          localDoneSet: {},
        ),
        isFalse,
      );
      expect(
        isInboxItemDone(
          row,
          markerOf: (key) => key == 'thread:$root' ? 50 : null,
          localUnreadOverrides: {},
          localDoneSet: {},
        ),
        isTrue,
      );

      session.emit(
        NostrEvent(
          id: 'delete',
          pubkey: 'alice',
          createdAt: 60,
          kind: 5,
          tags: const [
            ['h', _channelA],
            ['e', 'live'],
          ],
          content: '',
          sig: '',
        ),
      );
      expect(
        container.read(inboxItemsProvider).single.groupItems.map((e) => e.id),
        unorderedEquals(['cold', 'mention']),
      );
      // A late replay cannot resurrect the deleted Activity item.
      session.emit(reply('live', 50));
      expect(
        container.read(inboxItemsProvider).single.groupItems,
        hasLength(2),
      );

      // Identity changes reset the same registry, not an independent feed cache.
      container.read(_testPubkeyProvider.notifier).set('other-user');
      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      expect(container.read(inboxItemsProvider), isEmpty);
    },
  );
}
