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

      // The addressed feed keeps its own copy of a mention; the deletion
      // removes that row too, not only the channel registry's.
      session.emit(
        NostrEvent(
          id: 'delete-mention',
          pubkey: 'alice',
          createdAt: 61,
          kind: 5,
          tags: const [
            ['h', _channelA],
            ['e', 'mention'],
          ],
          content: '',
          sig: '',
        ),
      );
      expect(
        container.read(activityProvider).requireValue.all.map((e) => e.id),
        ['mention'],
      );
      expect(
        container.read(inboxItemsProvider).single.groupItems.map((e) => e.id),
        ['cold'],
      );

      // Identity changes reset the same registry, not an independent feed cache.
      container.read(_testPubkeyProvider.notifier).set('other-user');
      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      expect(container.read(inboxItemsProvider), isEmpty);
    },
  );

  test('received mentions and DMs reach Activity, the bell and the icon '
      'when history reads fail or miss them', () async {
    const dm = '33333333-3333-4333-8333-333333333333';
    NostrEvent message(
      String id,
      String channel,
      int at, {
      bool mention = false,
      bool quiet = false,
    }) => NostrEvent(
      id: id,
      pubkey: 'alice',
      createdAt: at,
      kind: 9,
      tags: [
        ['h', channel],
        if (mention) const ['p', 'me'],
        if (quiet) const ['audience', 'agents'],
      ],
      content: 'Message $id',
      sig: '',
    );
    final session = _FakeRelaySession(
      memberships: [
        _membership(_channelA, 'me'),
        _membership(dm, 'me', additionalPubkey: 'alice'),
      ],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(
          id: dm,
          name: 'alice',
          channelType: 'dm',
          visibility: 'private',
          participants: const ['me', 'alice'],
        ),
      ],
    );
    // The bell's local Activity state lives in preferences.
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = _buildContainer(
      session: session,
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        authProvider.overrideWith(_SignedInAuth.new),
        readStateProvider.overrideWith(_ReadyReadState.new),
      ],
    );
    addTearDown(container.dispose);
    await container.read(channelsProvider.future);
    await container.read(channelsProvider.notifier).waitForUnreadCatchUp();
    await container.read(activityProvider.future);
    final badge = container.listen(unreadBadgeProvider, (_, _) {});
    addTearDown(badge.close);
    await _settle();
    expect(container.read(unreadBadgeProvider), UnreadBadge.clear);

    List<String> activityIds() => [
      for (final row in container.read(inboxItemsProvider))
        for (final item in row.groupItems) item.id,
    ];
    Future<void> deliver(NostrEvent event) async {
      session.emit(event);
      await _settle();
    }

    // Only the live channel subscriptions still deliver: every history read
    // fails, so no Activity read can return these events, and the feed that
    // the reload publishes is incomplete.
    session.latestBatchError = StateError('history unavailable');
    session.messageHistoryError = StateError('history unavailable');
    container.invalidate(activityProvider);
    final failed = await container.read(activityProvider.future);
    expect(failed.complete, isFalse);

    // Ordinary channel traffic and quiet agent messages stay out.
    await deliver(message('ordinary', _channelA, 10));
    await deliver(message('quiet', _channelA, 11, mention: true, quiet: true));
    expect(activityIds(), isEmpty);
    expect(container.read(unreadBadgeProvider), UnreadBadge.pending);

    // A mention from a sender whose clock is far behind still counts, as
    // long as it is newer than the channel's read marker.
    await deliver(message('skewed-mention', _channelA, 12, mention: true));
    expect(activityIds(), ['skewed-mention']);
    expect(container.read(inboxHasUnreadProvider), isTrue);
    expect(container.read(unreadBadgeProvider), UnreadBadge.unread);

    // So does a DM message.
    await deliver(message('dm-message', dm, 13));
    expect(activityIds(), containsAll(['skewed-mention', 'dm-message']));
    final dmRow = container
        .read(inboxItemsProvider)
        .singleWhere((row) => row.groupItems.any((e) => e.id == 'dm-message'));
    expect(dmRow.groupItems.single.channelId, dm);

    // Reading both clears the bell. The icon holds rather than clears,
    // because the feed is still incomplete.
    (container.read(readStateProvider.notifier) as _ReadyReadState).read({
      _channelA: 12,
      dm: 13,
    });
    expect(container.read(inboxHasUnreadProvider), isFalse);
    expect(container.read(unreadBadgeProvider), UnreadBadge.pending);
  });

  test('a received mention raises the icon while Activity reloads, and the '
      'next account does not inherit it', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, 'me')],
      metadata: [_meta(id: _channelA, name: 'general')],
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = _buildContainer(
      session: session,
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        authProvider.overrideWith(_SignedInAuth.new),
        readStateProvider.overrideWith(_ReadyReadState.new),
      ],
    );
    addTearDown(container.dispose);
    await container.read(channelsProvider.future);
    await container.read(channelsProvider.notifier).waitForUnreadCatchUp();
    await container.read(activityProvider.future);
    final badge = container.listen(unreadBadgeProvider, (_, _) {});
    addTearDown(badge.close);
    await _settle();
    expect(container.read(unreadBadgeProvider), UnreadBadge.clear);

    // Hold Activity's reload open, as after a reconnect.
    final reload = Completer<void>();
    addTearDown(() {
      if (!reload.isCompleted) reload.complete();
    });
    session.latestBatchGate = reload;
    container.invalidate(activityProvider);
    await _settle();
    expect(container.read(activityProvider).isLoading, isTrue);
    expect(container.read(unreadBadgeProvider), UnreadBadge.pending);

    session.emit(
      NostrEvent(
        id: 'live-mention',
        pubkey: 'alice',
        createdAt: 50,
        kind: 9,
        tags: const [
          ['h', _channelA],
          ['p', 'me'],
        ],
        content: 'Look at this',
        sig: '',
      ),
    );
    await _settle();
    expect(container.read(activityProvider).isLoading, isTrue);
    expect(container.read(inboxHasUnreadProvider), isTrue);
    expect(container.read(unreadBadgeProvider), UnreadBadge.unread);

    // The channel list resets for another account, so the received mention
    // does not carry over to it.
    container.read(_testPubkeyProvider.notifier).set('other-user');
    await container.read(channelsProvider.future);
    await _settle();
    expect(container.read(observedInboxItemsProvider), isEmpty);
    expect(container.read(unreadBadgeProvider), isNot(UnreadBadge.unread));
  });
}

class _SignedInAuth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

class _ReadyReadState extends ReadStateNotifier {
  @override
  ReadStateState build() => const ReadStateState(
    isReady: true,
    pubkey: 'me',
    contexts: {},
    version: 1,
  );

  void read(Map<String, int> contexts) => state = ReadStateState(
    isReady: true,
    pubkey: 'me',
    contexts: contexts,
    version: state.version + 1,
  );
}
