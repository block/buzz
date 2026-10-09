part of '../channel_detail_page_test.dart';

void _loadingReviewTests() {
  for (final (count, target) in [(0, null), (3, null), (40, null), (40, 5)]) {
    testWidgets(
      'thread stays visible while loading $count replies target=$target',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final root = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Original message',
          createdAt: 1000,
        );
        final replies = [
          for (var i = 0; i < count; i++)
            _textMsg(
              id: 'reply-$i',
              pubkey: 'bob',
              content: 'Reply $i',
              createdAt: 1100 + i,
              extraTags: const [
                ['e', 'thread-root', '', 'reply'],
              ],
            ),
        ];
        final query = Completer<List<NostrEvent>>();
        await tester.pumpWidget(
          _buildTestable(
            messages: [root],
            pendingThreadReplies: {'thread-root': query.future},
            home: target == null
                ? null
                : ThreadDetailPage(
                    threadHead: formatTimeline([root]).single,
                    allMessages: formatTimeline([root]),
                    channelId: _channelId,
                    currentPubkey: 'self',
                    isMember: true,
                    isArchived: false,
                    initialMessageId: 'reply-$target',
                  ),
          ),
        );
        await tester.pumpAndSettle();
        if (target == null) {
          await tester.longPress(find.text('Original message').hitTestable());
          await tester.pumpAndSettle();
          await tester.tap(find.text('Reply').hitTestable());
          await tester.pumpAndSettle();
        }
        expect(find.text('Original message').hitTestable(), findsOneWidget);
        await tester.pump(const Duration(seconds: 2));
        expect(find.text('Original message').hitTestable(), findsOneWidget);
        query.complete(replies);
        for (var frame = 0; frame < 120; frame++) {
          await tester.pump(const Duration(milliseconds: 16));
          final gate = find.byKey(
            const ValueKey('thread-initial-viewport-gate'),
          );
          if (count <= 3) {
            expect(find.text('Original message').hitTestable(), findsOneWidget);
          }
          if (tester.widget<Opacity>(gate).opacity == 0 || count == 0) {
            expect(
              find.text('Original message').hitTestable(),
              findsOneWidget,
              reason:
                  'The original must remain usable while replies are positioned (frame $frame).',
            );
          } else {
            expect(
              find.text('Reply ${target ?? count - 1}').hitTestable(),
              findsOneWidget,
              reason:
                  'Reveal the reply list only at its final position (frame $frame).',
            );
          }
        }
        expect(
          tester
              .widget<Opacity>(
                find.byKey(const ValueKey('thread-initial-viewport-gate')),
              )
              .opacity,
          1,
        );
        if (count == 40) {
          final list = tester.widget<ScrollablePositionedList>(
            find.byKey(const ValueKey('thread-message-list')),
          );
          list.itemScrollController!.jumpTo(index: 0);
          await tester.pumpAndSettle();
          final before = tester.getTopLeft(find.text('Reply 0')).dy;
          await tester.drag(
            find.text('Original message').hitTestable(),
            const Offset(0, -100),
          );
          await tester.pumpAndSettle();
          final after = tester.getTopLeft(find.text('Reply 0')).dy;
          expect(
            after < before,
            isTrue,
            reason: 'Dragging the persistent head scrolls the reply list.',
          );
          list.itemScrollController!.jumpTo(index: 0);
          await tester.pumpAndSettle();
          final beforeWheel = tester.getTopLeft(find.text('Reply 0')).dy;
          await tester.sendEventToBinding(
            PointerScrollEvent(
              position: tester.getCenter(
                find.text('Original message').hitTestable(),
              ),
              scrollDelta: const Offset(0, 50),
            ),
          );
          await tester.pumpAndSettle();
          expect(
            tester.getTopLeft(find.text('Reply 0')).dy,
            lessThan(beforeWheel),
          );
          list.itemScrollController!.jumpTo(index: 0);
          await tester.pumpAndSettle();
          final semantics = tester.ensureSemantics();
          await tester.pump();
          final headNode = tester.getSemantics(
            find.byKey(const ValueKey('thread-head-scroll-semantics')),
          );
          final semanticsOwner = headNode.owner!;
          semanticsOwner.performAction(headNode.id, SemanticsAction.scrollDown);
          await tester.pumpAndSettle();
          expect(find.text('Original message').hitTestable(), findsNothing);
          final labels = <String>[];
          void collectLabels(SemanticsNode node) {
            labels.add(node.label);
            node.visitChildren((child) {
              collectLabels(child);
              return true;
            });
          }

          collectLabels(semanticsOwner.rootSemanticsNode!);
          expect(
            labels.any((label) => label.contains('Original message')),
            isFalse,
          );
          list.itemScrollController!.jumpTo(index: 1, alignment: 0.04);
          await tester.pumpAndSettle();
          final headPosition = list.itemPositionsNotifier!.itemPositions.value
              .singleWhere((item) => item.index == 0);
          expect(headPosition.itemTrailingEdge, greaterThan(0));
          expect(headPosition.itemTrailingEdge, lessThan(0.05));
          labels.clear();
          collectLabels(semanticsOwner.rootSemanticsNode!);
          expect(
            labels.any((label) => label.contains('Original message')),
            isFalse,
            reason: 'The frosted app bar covers the remaining head extent.',
          );
          list.itemScrollController!.jumpTo(index: 0);
          await tester.pumpAndSettle();
          labels.clear();
          collectLabels(semanticsOwner.rootSemanticsNode!);
          expect(
            labels.any((label) => label.contains('Original message')),
            isTrue,
          );
          semantics.dispose();
        }
      },
    );
  }

  for (final useSemantics in [false, true]) {
    testWidgets(
      'head non-drag scroll detaches hydration tail follow semantics=$useSemantics',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final root = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Original message',
          createdAt: 1000,
        );
        final replies = [
          for (var i = 0; i < 65; i++)
            _textMsg(
              id: 'reply-$i',
              pubkey: 'bob',
              content: 'Reply $i',
              createdAt: 1100 + i,
              extraTags: const [
                ['e', 'thread-root', '', 'reply'],
              ],
            ),
        ];
        final query = Completer<List<NostrEvent>>();
        final provisional = formatTimeline([root, ...replies.take(60)]);
        await tester.pumpWidget(
          _buildTestable(
            messages: [root],
            pendingThreadReplies: {'thread-root': query.future},
            home: ThreadDetailPage(
              threadHead: provisional.first,
              allMessages: provisional,
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
            ),
          ),
        );
        await tester.pumpAndSettle();
        final semantics = tester.ensureSemantics();
        await tester.pump();
        if (useSemantics) {
          final node = tester.getSemantics(
            find.byKey(const ValueKey('thread-head-scroll-semantics')),
          );
          node.owner!.performAction(node.id, SemanticsAction.scrollDown);
        } else {
          await tester.sendEventToBinding(
            PointerScrollEvent(
              position: tester.getCenter(
                find.text('Original message').hitTestable(),
              ),
              scrollDelta: const Offset(0, 50),
            ),
          );
        }
        await tester.pumpAndSettle();
        expect(find.text('Reply 59').hitTestable(), findsNothing);
        final list = tester.widget<ScrollablePositionedList>(
          find.byKey(const ValueKey('thread-message-list')),
        );
        final visibleReply = list.itemPositionsNotifier!.itemPositions.value
            .firstWhere(
              (item) =>
                  item.index > 0 &&
                  item.itemLeadingEdge > 0 &&
                  item.itemTrailingEdge < 1,
            );
        final anchor = find.byKey(
          ValueKey('thread-message-group-reply-${visibleReply.index - 1}'),
        );
        final top = tester.getTopLeft(anchor).dy;
        query.complete(replies);
        await tester.pumpAndSettle();
        expect(anchor, findsOneWidget);
        expect(tester.getTopLeft(anchor).dy, closeTo(top, 0.5));
        tester.view.physicalSize = const Size(400, 720);
        await tester.pumpAndSettle();
        expect(tester.getTopLeft(anchor).dy, closeTo(top, 0.5));
        semantics.dispose();
      },
    );
  }

  testWidgets('tall original message remains scrollable after hydration', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final root = _textMsg(
      id: 'tall-root',
      pubkey: 'alice',
      content:
          '${List.generate(70, (i) => 'Head line $i').join('\n')}\nEnd of original message',
    );
    final replies = Completer<List<NostrEvent>>();
    await tester.pumpWidget(
      _buildTestable(
        messages: [root],
        pendingThreadReplies: {'tall-root': replies.future},
        home: ThreadDetailPage(
          threadHead: formatTimeline([root]).single,
          allMessages: formatTimeline([root]),
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pumpAndSettle();
    replies.complete([
      _textMsg(
        id: 'tall-reply',
        pubkey: 'bob',
        content: 'Reply after tall head',
        extraTags: const [
          ['e', 'tall-root', '', 'reply'],
        ],
      ),
    ]);
    await tester.pumpAndSettle();
    final list = tester.widget<ScrollablePositionedList>(
      find.byKey(const ValueKey('thread-message-list')),
    );
    list.itemScrollController!.jumpTo(index: 0);
    await tester.pumpAndSettle();
    final head = find.byType(MessageContent).first;
    expect(tester.getSize(head).height, greaterThan(800));
    final rect = tester.getRect(head);
    final bottomPoint = Offset(rect.center.dx, rect.bottom - 20);
    for (var i = 0; i < 9; i++) {
      await tester.dragFrom(const Offset(200, 500), const Offset(0, -180));
      await tester.pumpAndSettle();
    }
    final movedRect = tester.getRect(head);
    expect(movedRect.bottom, lessThan(750));
    expect(movedRect.bottom, greaterThan(100));
    expect(
      tester
          .hitTestOnBinding(Offset(movedRect.center.dx, movedRect.bottom - 20))
          .path
          .any((entry) => entry.target == tester.renderObject(head)),
      isTrue,
      reason:
          'The bottom of the tall original remains interactive, not clipped.',
    );
    expect(find.text('Reply after tall head').hitTestable(), findsOneWidget);
    expect(bottomPoint.dy, greaterThan(800));
  });

  for (final completesAfterDisposal in [false, true]) {
    testWidgets(
      'thread hydration owns one video preview lifecycle late=$completesAfterDisposal',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final root = _textMsg(
          id: 'video-root',
          pubkey: 'alice',
          content: '![video](https://example.com/head.mp4)',
          extraTags: const [
            [
              'imeta',
              'url https://example.com/head.mp4',
              'm video/mp4',
              'dim 320x180',
            ],
          ],
        );
        final replies = Completer<List<NostrEvent>>();
        final preview = Completer<LoadedVideoPreviewFrame?>();
        var loads = 0;
        var disposals = 0;
        await tester.pumpWidget(
          _buildTestable(
            messages: [root],
            pendingThreadReplies: {'video-root': replies.future},
            videoPreviewLoader: (_) {
              loads++;
              return preview.future;
            },
            home: ThreadDetailPage(
              threadHead: formatTimeline([root]).single,
              allMessages: formatTimeline([root]),
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
            ),
          ),
        );
        for (var frame = 0; frame < 10; frame++) {
          await tester.pump(const Duration(milliseconds: 16));
        }
        expect(loads, 1);
        final frameResource = LoadedVideoPreviewFrame(
          child: const SizedBox(),
          aspectRatio: 16 / 9,
          dispose: () async {
            disposals++;
          },
        );
        if (!completesAfterDisposal) {
          preview.complete(frameResource);
          await tester.pump();
        }
        replies.complete([
          _textMsg(
            id: 'video-reply',
            pubkey: 'bob',
            content: 'Reply to video',
            extraTags: const [
              ['e', 'video-root', '', 'reply'],
            ],
          ),
        ]);
        for (var frame = 0; frame < 30; frame++) {
          await tester.pump(const Duration(milliseconds: 16));
          expect(
            loads,
            1,
            reason: 'One head owns video initialization at frame $frame',
          );
          expect(
            find
                .byKey(
                  const ValueKey(
                    'message-media-video-preview:https://example.com/head.mp4',
                  ),
                )
                .hitTestable(),
            findsOneWidget,
          );
        }
        final semantics = tester.ensureSemantics();
        await tester.pump();
        final video = find.byKey(
          const ValueKey(
            'message-media-video-preview:https://example.com/head.mp4',
          ),
        );
        final node = tester.getSemantics(video);
        var semanticRect = node.rect;
        for (var ancestor = node; ;) {
          final transform = ancestor.transform;
          if (transform != null) {
            semanticRect = MatrixUtils.transformRect(transform, semanticRect);
          }
          final parent = ancestor.parent;
          if (parent == null) break;
          ancestor = parent;
        }
        expect(semanticRect.contains(tester.getCenter(video)), isTrue);
        semantics.dispose();
        await tester.pumpWidget(const SizedBox());
        await tester.pump();
        if (completesAfterDisposal) preview.complete(frameResource);
        await tester.pump();
        expect(
          disposals,
          1,
          reason: 'Late completion releases its single resource',
        );
      },
    );
  }

  testWidgets('loaded page rebuild formats the timeline once', (tester) async {
    var transforms = 0;
    debugOnFormatTimeline = () => transforms++;
    addTearDown(() => debugOnFormatTimeline = null);
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _textMsg(id: 'one', pubkey: 'alice', content: 'Loaded message'),
        ],
      ),
    );
    await tester.pumpAndSettle();
    for (var i = 0; i < 3; i++) {
      transforms = 0;
      tester.element(find.byType(ChannelDetailPage)).markNeedsBuild();
      await tester.pump();
      expect(transforms, 1);
    }
  });

  testWidgets('reconnect snapshot puts newest cached media at the bottom', (
    tester,
  ) async {
    final relay = _ReconnectingRelaySession(
      initialStatus: SessionStatus.connected,
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          for (var i = 0; i < 2; i++)
            _textMsg(
              id: 'image-$i',
              pubkey: 'alice',
              createdAt: 1000 + i,
              content: '![photo](https://example.com/$i.jpg)',
              extraTags: [
                [
                  'imeta',
                  'url https://example.com/$i.jpg',
                  'm image/jpeg',
                  'dim 400x100',
                ],
              ],
            ),
        ],
        relaySessionNotifier: relay,
      ),
    );
    await tester.pumpAndSettle();
    final olderPreview = find.byKey(
      const ValueKey('message-media-image-preview:https://example.com/0.jpg'),
    );
    final newerPreview = find.byKey(
      const ValueKey('message-media-image-preview:https://example.com/1.jpg'),
    );
    expect(
      tester.getCenter(olderPreview).dy,
      lessThan(tester.getCenter(newerPreview).dy),
    );
    relay.setReconnecting();
    await tester.pump();
    await tester.pump(const Duration(seconds: 3));
    final olderShape = find.byKey(
      const ValueKey('message-skeleton-image:https://example.com/0.jpg'),
    );
    final newerShape = find.byKey(
      const ValueKey('message-skeleton-image:https://example.com/1.jpg'),
    );
    expect(
      tester.getCenter(olderShape).dy,
      lessThan(tester.getCenter(newerShape).dy),
    );
    final list = find
        .ancestor(of: newerShape, matching: find.byType(ListView))
        .first;
    expect(tester.widget<ListView>(list).reverse, isTrue);
    expect(
      tester.getCenter(newerShape).dy,
      greaterThan(tester.getSize(find.byType(Scaffold).first).height / 2),
    );
    relay.connect();
    await tester.pumpAndSettle();
    expect(
      tester.getCenter(olderPreview).dy,
      lessThan(tester.getCenter(newerPreview).dy),
    );
  });

  testWidgets(
    'same loading cycle updates connection semantics without changing shapes',
    (tester) async {
      final semantics = tester.ensureSemantics();
      final relay = _ReconnectingRelaySession(
        initialStatus: SessionStatus.connecting,
      );
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: _FakeMessagesNotifier(
            const [],
            hasLoadedMessages: false,
          ),
          relaySessionNotifier: relay,
        ),
      );
      await tester.pump();
      final key = find.byKey(const Key('channel-detail-connection-skeleton'));
      expect(tester.getSemantics(key).label, 'Connecting');
      final reveal = tester.widget<SkeletonReveal>(find.byType(SkeletonReveal));
      expect(reveal.loading, isTrue);
      relay.setReconnecting();
      await tester.pump();
      expect(
        tester.widget<SkeletonReveal>(find.byType(SkeletonReveal)).loading,
        isTrue,
      );
      expect(tester.getSemantics(key).label, 'Reconnecting');
      semantics.dispose();
    },
  );
}
