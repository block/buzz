part of 'channel_detail_page_test.dart';

void threadReplyRefreshTests() {
  const args = ThreadRepliesArgs(channelId: _channelId, rootId: 'refresh-root');
  final root = _textMsg(
    id: args.rootId,
    pubkey: 'alice',
    content: 'Refresh root',
  );
  final reply = _textMsg(
    id: 'cached-reply',
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
    TextScaler textScaler = TextScaler.noScaling,
  }) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [root],
        textScaler: textScaler,
        threadReplyLoaders: {args.rootId: load},
        providerRetry: (_, _) =>
            retry ? const Duration(milliseconds: 200) : null,
      ),
    );
    await tester.pumpAndSettle();
    final context = tester.element(find.byType(ChannelDetailPage));
    return (Navigator.of(context), ProviderScope.containerOf(context));
  }

  void open(NavigatorState navigator) {
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
        ),
      ),
    );
  }

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
          find.byKey(const ValueKey('thread-message-group-cached-reply')),
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
      find.byKey(const ValueKey('thread-message-group-cached-reply')),
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
}
