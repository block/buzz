part of 'channel_detail_page_test.dart';

void threadReplyRefreshTests() {
  const args = ThreadRepliesArgs(
    channelId: _channelId,
    rootId: threadIdRefreshDashRoot,
  );
  final root = _textMsg(
    id: args.rootId,
    pubkey: 'alice',
    content: 'Refresh root',
  );
  final reply = _textMsg(
    id: threadIdR1,
    pubkey: 'bob',
    content: 'Cached reply',
    createdAt: 1001,
    extraTags: [
      ['e', args.rootId, '', 'reply'],
    ],
  );

  Future<(NavigatorState, ProviderContainer)> mount(
    WidgetTester tester,
    Future<List<NostrEvent>> Function() load, {
    bool retry = false,
    bool productionRetry = false,
    TextScaler textScaler = TextScaler.noScaling,
    Future<List<NostrEvent>?>? savedThreadPreview,
    _FakeMessagesNotifier? messagesNotifier,
    ReadStateNotifier? readStateNotifier,
  }) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [root],
        savedThreadPreview: savedThreadPreview,
        messagesNotifier: messagesNotifier,
        readStateNotifier: readStateNotifier,
        textScaler: textScaler,
        threadReplyLoaders: {args.rootId: load},
        providerRetry: productionRetry
            ? relayProviderRetry
            : (_, _) => retry ? const Duration(milliseconds: 200) : null,
      ),
    );
    await tester.pumpAndSettle();
    final context = tester.element(find.byType(ChannelDetailPage));
    return (Navigator.of(context), ProviderScope.containerOf(context));
  }

  void open(NavigatorState navigator, {String? initialMessageId}) {
    final head = formatTimeline([root]).single;
    navigator.push(
      MaterialPageRoute<void>(
        builder: (_) => ThreadDetailPage(
          threadHead: head,
          allMessages: [head],
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
          initialMessageId: initialMessageId,
        ),
      ),
    );
  }

  testWidgets('an offline link opens a saved root outside channel history', (
    tester,
  ) async {
    final preview = Completer<List<NostrEvent>?>();
    var offline = true;
    final fresh = _textMsg(
      id: 'fresh-reply',
      pubkey: 'bob',
      content: 'Fresh relay reply',
      createdAt: 1002,
      extraTags: [
        ['e', args.rootId, '', 'reply'],
      ],
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: const [],
        savedThreadPreview: preview.future,
        initialThreadRootId: args.rootId,
        initialMessageId: reply.id,
        disableRetries: true,
        threadReplyLoaders: {
          args.rootId: () async {
            if (offline) throw const SocketException('Offline');
            return [fresh];
          },
        },
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byType(ThreadDetailPage), findsNothing);

    preview.complete([root, reply]);
    await tester.pumpAndSettle();
    final route = find.byType(ThreadDetailPage);
    expect(route, findsOneWidget);
    expect(find.text(root.content), findsOneWidget);
    expect(find.text(reply.content), findsOneWidget);
    expect(tester.widget<ThreadDetailPage>(route).initialMessageId, reply.id);

    final container = ProviderScope.containerOf(tester.element(route));
    offline = false;
    container.invalidate(threadRepliesProvider(args));
    await tester.pumpAndSettle();
    expect(find.text(root.content), findsOneWidget);
    expect(find.text(reply.content), findsNothing);
    expect(find.text(fresh.content), findsOneWidget);
  });

  testWidgets(
    'thread refresh keeps cached replies through retry and recovery',
    (tester) async {
      var calls = 0;
      final refresh = Completer<List<NostrEvent>>();
      final retry = Completer<List<NostrEvent>>();
      final (navigator, container) = await mount(tester, () {
        calls++;
        return switch (calls) {
          1 => Future.value([reply]),
          2 => refresh.future,
          _ => retry.future,
        };
      }, retry: true);
      open(navigator);
      await tester.pumpAndSettle();
      expect(find.text('1 reply'), findsOneWidget);
      container.invalidate(threadRepliesProvider(args));
      await tester.pump();
      expect(find.text('1 reply'), findsOneWidget);
      refresh.completeError(Exception('Transient refresh failure'));
      await tester.pump();
      for (var frame = 0; frame < 20; frame++) {
        await tester.pump(const Duration(milliseconds: 16));
        expect(find.text('0 replies'), findsNothing, reason: 'frame $frame');
        expect(
          find.byKey(const ValueKey('thread-message-group-$threadIdR1')),
          findsOneWidget,
        );
        expect(container.read(threadRepliesProvider(args)).value, [reply]);
      }
      expect(calls, 3);
      retry.complete([
        reply,
        _textMsg(
          id: 'fresh-reply',
          pubkey: 'bob',
          content: 'Fresh reply',
          createdAt: 1002,
          extraTags: [
            ['e', args.rootId, '', 'reply'],
          ],
        ),
      ]);
      await tester.pumpAndSettle();
      expect(find.text('2 replies'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('thread-message-group-fresh-reply')),
        findsOneWidget,
      );
    },
  );

  testWidgets('thread refresh error keeps cached replies visible', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    var calls = 0;
    final refresh = Completer<List<NostrEvent>>();
    tester.view.physicalSize = const Size(320, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final (navigator, container) = await mount(
      tester,
      () => ++calls == 1 ? Future.value([reply]) : refresh.future,
      textScaler: const TextScaler.linear(2),
    );
    open(navigator);
    await tester.pumpAndSettle();
    container.invalidate(threadRepliesProvider(args));
    await tester.pump();
    refresh.completeError(Exception('Refresh failed'));
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('thread-message-group-$threadIdR1')),
      findsOneWidget,
    );
    expect(find.text('0 replies'), findsNothing);
    expect(find.text('1 reply · Couldn’t refresh'), findsOneWidget);
    expect(
      tester
          .getSemantics(find.text('1 reply · Couldn’t refresh'))
          .getSemanticsData()
          .flagsCollection
          .isLiveRegion,
      isTrue,
    );
    expect(tester.takeException(), isNull);
    expect(container.read(threadRepliesProvider(args)).hasError, isTrue);
    semantics.dispose();
  });

  testWidgets('thread reopen shows loading until the fresh query completes', (
    tester,
  ) async {
    var calls = 0;
    final reopened = Completer<List<NostrEvent>>();
    final (navigator, _) = await mount(
      tester,
      () => ++calls == 1 ? Future.value([reply]) : reopened.future,
    );
    open(navigator);
    await tester.pumpAndSettle();
    navigator.pop();
    await tester.pumpAndSettle();
    open(navigator);
    await tester.pumpAndSettle();
    expect(calls, 2);
    expect(find.text('0 replies'), findsNothing);
    expect(find.text('Loading replies…'), findsOneWidget);
    reopened.complete([reply]);
    await tester.pumpAndSettle();
    expect(find.text('1 reply'), findsOneWidget);
    expect(find.text('Loading replies…'), findsNothing);
  });

  testWidgets('thread initial error is distinct from an empty thread', (
    tester,
  ) async {
    final query = Completer<List<NostrEvent>>();
    final (navigator, _) = await mount(tester, () => query.future);
    open(navigator);
    await tester.pumpAndSettle();
    query.completeError(Exception('Initial load failed'));
    await tester.pumpAndSettle();
    expect(find.text('0 replies'), findsNothing);
    expect(find.text('Couldn’t load replies'), findsOneWidget);
  });
  testWidgets('thread successful empty query displays zero replies', (
    tester,
  ) async {
    final (navigator, _) = await mount(tester, () async => []);
    open(navigator);
    await tester.pumpAndSettle();
    expect(find.text('0 replies'), findsOneWidget);
    expect(find.text('Loading replies…'), findsNothing);
    expect(find.text('Couldn’t load replies'), findsNothing);
  });

  testWidgets(
    'saved replies stay provisional until relay history replaces them',
    (tester) async {
      final query = Completer<List<NostrEvent>>();
      final liveReply = _textMsg(
        id: 'live-preview-reply',
        pubkey: 'carol',
        content: 'Live preview reply',
        createdAt: 1002,
        extraTags: [
          ['e', args.rootId, '', 'reply'],
        ],
      );
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {},
          version: 0,
        ),
      );
      final (navigator, _) = await mount(
        tester,
        () => query.future,
        savedThreadPreview: Future.value([reply]),
        messagesNotifier: _FakeMessagesNotifier([root, liveReply]),
        readStateNotifier: readState,
      );
      open(navigator);
      await tester.pumpAndSettle();
      expect(find.text('Showing saved replies.'), findsOneWidget);
      expect(find.text('1 reply'), findsNothing);
      expect(findRichText('Cached reply'), findsOneWidget);
      await tester.pump(const Duration(milliseconds: 350));
      expect(readState.markedContexts, contains(msgContextKey(liveReply.id)));
      expect(
        readState.markedContexts,
        isNot(contains(msgContextKey(reply.id))),
      );
      expect(
        readState.markedContexts,
        isNot(contains(threadContextKey(args.rootId))),
      );

      query.complete([]);
      await tester.pumpAndSettle();
      expect(find.text('1 reply'), findsOneWidget);
      expect(findRichText('Cached reply'), findsNothing);
    },
  );

  testWidgets(
    'saved thread opens at Latest and preserves the reading position on handoff',
    (tester) async {
      tester.view.physicalSize = const Size(400, 680);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      NostrEvent row(int index) => _textMsg(
        id: 'handoff-reply-$index',
        pubkey: 'bob',
        content: 'Cached reply $index',
        createdAt: 1100 + index,
        extraTags: [
          ['e', args.rootId, '', 'reply'],
        ],
      );
      final preview = List.generate(40, row);
      final query = Completer<List<NostrEvent>>();
      final (navigator, _) = await mount(
        tester,
        () => query.future,
        savedThreadPreview: Future.value(preview),
      );
      open(navigator);
      await tester.pumpAndSettle();

      final list = find.byKey(const ValueKey('thread-message-list'));
      final gate = find.byKey(const ValueKey('thread-initial-viewport-gate'));
      final latest = find.byKey(
        const ValueKey('thread-message-group-handoff-reply-39'),
      );
      expect(
        tester.getBottomLeft(latest).dy,
        lessThanOrEqualTo(
          tester.getTopLeft(find.byKey(const ValueKey('composer-surface'))).dy,
        ),
      );
      await tester.drag(list, const Offset(0, 480));
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('thread-jump-to-latest')),
        findsOneWidget,
      );
      final appBarBottom = frostedAppBarHeight(tester.element(list));
      final anchor =
          List.generate(
            preview.length,
            (index) => find.byKey(
              ValueKey('thread-message-group-handoff-reply-$index'),
            ),
          ).firstWhere(
            (row) =>
                row.evaluate().isNotEmpty &&
                tester.getBottomLeft(row).dy > appBarBottom,
          );
      final before = tester.getTopLeft(anchor).dy;
      query.complete([row(-2), row(-1), ...preview, row(40)]);
      await tester.pump();
      expect(tester.widget<Opacity>(gate).opacity, 1);
      await tester.pumpAndSettle();
      expect(tester.widget<Opacity>(gate).opacity, 1);
      expect(tester.getTopLeft(anchor).dy, closeTo(before, 2));
    },
  );

  testWidgets('live nested links stay available beside saved-only replies', (
    tester,
  ) async {
    final parent = _textMsg(
      id: threadIdParent,
      pubkey: 'bob',
      content: 'Live parent',
      createdAt: 1001,
      extraTags: [
        ['e', args.rootId, '', 'reply'],
      ],
    );
    final child = _textMsg(
      id: 'live-child',
      pubkey: 'bob',
      content: 'Live child',
      createdAt: 1002,
      extraTags: [
        ['e', args.rootId, '', 'root'],
        ['e', parent.id, '', 'reply'],
      ],
    );
    final notifier = _FakeMessagesNotifier([root, parent, child]);
    final query = Completer<List<NostrEvent>>();
    final (navigator, _) = await mount(
      tester,
      () => query.future,
      savedThreadPreview: Future.value([reply]),
      messagesNotifier: notifier,
    );
    open(navigator);
    await tester.pumpAndSettle();
    expect(find.text('Showing saved replies.'), findsOneWidget);
    await tester.tap(
      find.byKey(ValueKey('nested-thread-summary-${parent.id}')),
    );
    await tester.pumpAndSettle();
    expect(findRichText('Live child'), findsOneWidget);
    expect(find.text('Showing saved replies.'), findsNothing);
  });

  testWidgets(
    'a live deletion hides a saved reply before relay history arrives',
    (tester) async {
      final query = Completer<List<NostrEvent>>();
      final notifier = _FakeMessagesNotifier([root]);
      final (navigator, _) = await mount(
        tester,
        () => query.future,
        savedThreadPreview: Future.value([reply]),
        messagesNotifier: notifier,
      );
      open(navigator);
      await tester.pumpAndSettle();
      expect(findRichText('Cached reply'), findsOneWidget);

      notifier.setMessages([
        root,
        NostrEvent(
          id: 'deletion',
          pubkey: reply.pubkey,
          createdAt: 1002,
          kind: EventKind.deletion,
          tags: [
            ['e', reply.id],
          ],
          content: '',
          sig: '',
        ),
      ]);
      await tester.pumpAndSettle();
      expect(findRichText('Cached reply'), findsNothing);
      expect(find.text('Showing saved replies.'), findsNothing);
      expect(find.text('Loading replies…'), findsOneWidget);

      query.completeError(Exception('offline'));
      await tester.pumpAndSettle();
      expect(find.text('Couldn’t load replies'), findsOneWidget);
      expect(find.byTooltip('Retry'), findsOneWidget);
    },
  );

  testWidgets('a channel refusal clears a displayed saved reply', (
    tester,
  ) async {
    final query = Completer<List<NostrEvent>>();
    final (navigator, _) = await mount(
      tester,
      () => query.future,
      savedThreadPreview: Future.value([reply]),
    );
    open(navigator);
    await tester.pumpAndSettle();
    expect(findRichText('Cached reply'), findsOneWidget);

    query.completeError(Exception('restricted: not a channel member'));
    await tester.pumpAndSettle();
    final unavailable = find.text('This thread is unavailable');
    expect(unavailable, findsOneWidget);
    final semantics = tester.ensureSemantics();
    try {
      await tester.pump();
      expect(
        tester
            .getSemantics(unavailable)
            .getSemanticsData()
            .flagsCollection
            .isLiveRegion,
        isTrue,
      );
    } finally {
      semantics.dispose();
    }
    expect(findRichText('Cached reply'), findsNothing);
  });

  for (final diskArrivesFirst in [true, false]) {
    testWidgets(
      'offline thread highlights a saved target (disk first: $diskArrivesFirst)',
      (tester) async {
        final query = Completer<List<NostrEvent>>();
        final disk = Completer<List<NostrEvent>?>();
        final (navigator, _) = await mount(
          tester,
          () => query.future,
          savedThreadPreview: disk.future,
        );
        open(navigator, initialMessageId: reply.id);
        await tester.pumpAndSettle();

        if (diskArrivesFirst) {
          disk.complete([reply]);
        } else {
          query.completeError(Exception('offline'));
        }
        await tester.pumpAndSettle();
        final target = find.byKey(ValueKey('thread-message-${reply.id}'));
        if (diskArrivesFirst) {
          final before =
              tester.widget<DecoratedBox>(target).decoration as BoxDecoration;
          expect(before.color, Colors.transparent);
          query.completeError(Exception('offline'));
        } else {
          expect(target, findsNothing);
          disk.complete([reply]);
        }

        for (var i = 0; i < 8; i++) {
          await tester.pump();
        }
        await tester.pump(const Duration(milliseconds: 50));
        await tester.pump(const Duration(milliseconds: 150));
        final highlight =
            tester.widget<DecoratedBox>(target).decoration as BoxDecoration;
        expect(highlight.color!.a, greaterThan(0));
      },
    );
  }

  testWidgets('a linked saved reply highlights during production retries', (
    tester,
  ) async {
    final retry = Completer<List<NostrEvent>>();
    var calls = 0;
    final (navigator, container) = await mount(
      tester,
      () => ++calls == 1
          ? Future.error(const SocketException('Offline'))
          : retry.future,
      productionRetry: true,
      savedThreadPreview: Future.value([reply]),
    );
    open(navigator, initialMessageId: reply.id);
    await tester.pumpAndSettle();
    await tester.pump(const Duration(milliseconds: 150));

    final state = container.read(threadRepliesProvider(args));
    expect(state.isLoading, isTrue);
    expect(state.hasValue, isFalse);
    final target = find.byKey(ValueKey('thread-message-${reply.id}'));
    final highlight =
        tester.widget<DecoratedBox>(target).decoration as BoxDecoration;
    expect(highlight.color!.a, greaterThan(0));
  });

  testWidgets('scrolling saved replies supersedes a pending link jump', (
    tester,
  ) async {
    final preview = List.generate(
      40,
      (index) => _textMsg(
        id: 'linked-preview-reply-$index',
        pubkey: 'bob',
        content: 'Saved reply $index',
        createdAt: 1100 + index,
        extraTags: [
          ['e', args.rootId, '', 'reply'],
        ],
      ),
    );
    final query = Completer<List<NostrEvent>>();
    final (navigator, _) = await mount(
      tester,
      () => query.future,
      savedThreadPreview: Future.value(preview),
    );
    open(navigator, initialMessageId: preview.first.id);
    await tester.pumpAndSettle();

    final list = find.byKey(const ValueKey('thread-message-list'));
    await tester.drag(list, const Offset(0, -700));
    await tester.pumpAndSettle();
    final first = findRichText(preview.first.content).hitTestable();
    expect(first, findsNothing);

    query.complete(preview);
    await tester.pumpAndSettle();
    expect(first, findsNothing);
  });

  testWidgets('a deleted thread head keeps the saved reply status and Retry', (
    tester,
  ) async {
    final deletion = NostrEvent(
      id: 'deleted-thread-head',
      pubkey: root.pubkey,
      createdAt: 1002,
      kind: EventKind.deletion,
      tags: [
        ['e', root.id],
      ],
      content: '',
      sig: '',
    );
    final (navigator, _) = await mount(
      tester,
      () => Future.error(const SocketException('Offline')),
      savedThreadPreview: Future.value([reply, deletion]),
    );
    open(navigator, initialMessageId: reply.id);
    await tester.pumpAndSettle();

    expect(find.text('This message was deleted'), findsOneWidget);
    expect(find.text('Showing saved replies. Refresh failed.'), findsOneWidget);
    expect(find.byTooltip('Retry'), findsOneWidget);
    expect(findRichText(reply.content), findsOneWidget);
  });

  testWidgets('thread loads an edit made to a reply before it opened', (
    tester,
  ) async {
    // The real thread query runs; the relay session is faked. It sends the
    // reply's earlier edit only to a query that asks for aux events.
    final edit = NostrEvent(
      id: 'cached-reply-edit',
      pubkey: 'bob',
      createdAt: 1002,
      kind: EventKind.streamMessageEdit,
      tags: [
        const ['h', _channelId],
        ['e', reply.id],
      ],
      content: 'Fixed reply',
      sig: '',
    );
    final session = _ThreadAuxRelaySession([reply], [edit]);
    await tester.pumpWidget(
      _buildTestable(messages: [root], relaySessionNotifier: session),
    );
    await tester.pumpAndSettle();
    open(Navigator.of(tester.element(find.byType(ChannelDetailPage))));
    await tester.pumpAndSettle();
    final thread = find.byType(ThreadDetailPage);
    expect(
      find.descendant(
        of: thread,
        matching: findRichText('Fixed reply (edited)'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(of: thread, matching: findRichText('Cached reply')),
      findsNothing,
    );
  });
}

class _ThreadAuxRelaySession extends RelaySessionNotifier {
  _ThreadAuxRelaySession(this.replies, this.aux);
  final List<NostrEvent> replies;
  final List<NostrEvent> aux;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [
    for (final filter in filters)
      if (filter.extensions.containsKey('depth_limit')) ...[
        ...replies,
        if (filter.extensions['include_aux'] == true) ...aux,
      ],
  ];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];
}
