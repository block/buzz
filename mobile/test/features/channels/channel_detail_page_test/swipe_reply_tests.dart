part of '../channel_detail_page_test.dart';

void swipeReplyTests() {
  // Solo agents answer at top level, so a Solo chat starts no threads.
  Channel soloChannel() => Channel(
    id: _channelId,
    name: 'Solo',
    channelType: 'stream',
    visibility: 'private',
    labels: const ['buzz.solo'],
    description: '',
    createdBy: 'self',
    createdAt: DateTime(2026),
    memberCount: 2,
    isMember: true,
  );

  testWidgets('Solo chat offers no reply by swipe or message actions', (
    tester,
  ) async {
    final channel = soloChannel();
    final events = [
      _textMsg(id: threadIdRoot, pubkey: 'agent', content: 'Agent answer'),
    ];
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        messages: events,
        channel: channel,
        personalChat: true,
        organization: Stream.value(const MeGroups()),
        home: ChannelDetailPage(channel: channel),
      ),
    );
    await tester.pumpAndSettle();
    final row = find.byKey(const ValueKey('message-row-$threadIdRoot'));
    final bubble = find
        .descendant(of: row, matching: find.byType(LocalSendBubbleSurface))
        .first;
    final initial = tester.getTopLeft(row);
    final gesture = await tester.startGesture(tester.getCenter(bubble));
    await gesture.moveBy(const Offset(25, 0));
    await gesture.moveBy(const Offset(150, 0));
    await tester.pump();
    expect(tester.getTopLeft(row), initial);
    expect(find.byKey(const ValueKey('swipe-reply-circle')), findsNothing);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(find.byType(ThreadDetailPage), findsNothing);

    await tester.longPress(bubble);
    await tester.pumpAndSettle();
    expect(find.text('Copy link'), findsOneWidget);
    expect(find.text('Reply'), findsNothing);
  });

  testWidgets('Solo thread is read-only', (tester) async {
    final channel = soloChannel();
    final events = [
      _textMsg(id: threadIdRoot, pubkey: 'self', content: 'Old thread head'),
    ];
    final messages = formatTimeline(events);
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        messages: events,
        channel: channel,
        personalChat: true,
        organization: Stream.value(const MeGroups()),
        home: ThreadDetailPage(
          threadHead: messages.first,
          allMessages: messages,
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Old thread head'), findsOneWidget);
    expect(find.byType(ComposeBar), findsNothing);
  });

  for (final thread in [false, true]) {
    for (final own in [false, true]) {
      testWidgets(
        'left drag opens existing reply destination, thread=$thread own=$own',
        (tester) async {
          final events = [
            _textMsg(
              id: threadIdRoot,
              pubkey: own ? 'self' : 'alice',
              content: 'Swipe this message\nA second line\nAnd a third line',
            ),
          ];
          final messages = formatTimeline(events);
          var navigationDrags = 0;
          await tester.pumpWidget(
            _buildTestable(
              signingPubkey: 'self',
              messages: events,
              chatColor: 'magenta',
              home: GestureDetector(
                onHorizontalDragUpdate: (_) => navigationDrags++,
                child: thread
                    ? ThreadDetailPage(
                        threadHead: messages.first,
                        allMessages: messages,
                        channelId: _channelId,
                        currentPubkey: 'self',
                        isMember: true,
                        isArchived: false,
                      )
                    : ChannelDetailPage(channel: _testChannel),
              ),
            ),
          );
          await tester.pumpAndSettle();
          final row = find.byKey(
            ValueKey(
              '${thread ? 'thread-message-row' : 'message-row'}-$threadIdRoot',
            ),
          );
          final initial = tester.getTopLeft(row);
          final haptics = <MethodCall>[];
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            SystemChannels.platform,
            (call) async {
              if (call.method == 'HapticFeedback.vibrate') haptics.add(call);
              return null;
            },
          );
          addTearDown(
            () => tester.binding.defaultBinaryMessenger
                .setMockMethodCallHandler(SystemChannels.platform, null),
          );
          // Metadata, avatar and empty row space never arm a reply, even when
          // the pointer later crosses into the message.
          final bubbleRect = tester.getRect(
            find
                .descendant(
                  of: row,
                  matching: find.byType(LocalSendBubbleSurface),
                )
                .first,
          );
          for (final start in [
            Offset(tester.getRect(row).left + 2, bubbleRect.center.dy),
            Offset(tester.getRect(row).right - 2, bubbleRect.center.dy),
            Offset(bubbleRect.left + 4, tester.getRect(row).top + 2),
          ]) {
            if (bubbleRect.contains(start)) continue;
            final previousNavigationDrags = navigationDrags;
            final outside = await tester.startGesture(start);
            await outside.moveBy(const Offset(-25, 0));
            await outside.moveBy(const Offset(-100, 0));
            await tester.pump();
            expect(tester.getTopLeft(row), initial);
            expect(haptics, isEmpty);
            expect(navigationDrags, greaterThan(previousNavigationDrags));
            await outside.up();
            await tester.pumpAndSettle();
            expect(Navigator.of(tester.element(row)).canPop(), isFalse);
          }
          // A rightward drag on the bubble is left to swipe-back navigation.
          final bubbleCenter = bubbleRect.center;
          final previousNavigationDrags = navigationDrags;
          final back = await tester.startGesture(bubbleCenter);
          await back.moveBy(const Offset(25, 0));
          await back.moveBy(const Offset(100, 0));
          await tester.pump();
          expect(tester.getTopLeft(row), initial);
          expect(haptics, isEmpty);
          expect(navigationDrags, greaterThan(previousNavigationDrags));
          await back.up();
          await tester.pumpAndSettle();
          expect(Navigator.of(tester.element(row)).canPop(), isFalse);
          final gesture = await tester.startGesture(bubbleCenter);
          await gesture.moveBy(const Offset(-25, 0));
          await gesture.moveBy(const Offset(-40, 0));
          await tester.pump();
          expect(tester.getTopLeft(row).dx, lessThan(initial.dx));
          expect(haptics, isEmpty);
          final partialCircle = tester.widget<DecoratedBox>(
            find.byKey(const ValueKey('swipe-reply-circle')),
          );
          expect(
            (partialCircle.decoration as BoxDecoration).color,
            Colors.transparent,
          );
          final ring = tester.widget<CircularProgressIndicator>(
            find.descendant(
              of: find.byKey(const ValueKey('swipe-reply-circle')),
              matching: find.byType(CircularProgressIndicator),
            ),
          );
          expect(ring.strokeWidth, 2);
          expect(ring.value, inExclusiveRange(0, 1));
          await gesture.moveBy(const Offset(-45, 0));
          await tester.pump();
          expect(haptics, hasLength(1));
          await gesture.moveBy(const Offset(-10, 0));
          await tester.pump();
          expect(haptics, hasLength(1));
          final armedPosition = tester.getTopLeft(row);
          await gesture.moveBy(const Offset(-160, 0));
          await tester.pump();
          final resistedPosition = tester.getTopLeft(row);
          expect(resistedPosition.dx, lessThan(armedPosition.dx));
          expect(resistedPosition.dx, greaterThan(armedPosition.dx - 40));
          final circle = find.byKey(const ValueKey('swipe-reply-circle'));
          final circleRect = tester.getRect(circle);
          // The armed slide uncovers the feedback, which stays on screen.
          expect(
            circleRect.left,
            greaterThan(
              tester
                  .getRect(
                    find
                        .descendant(
                          of: row,
                          matching: find.byType(LocalSendBubbleSurface),
                        )
                        .first,
                  )
                  .right,
            ),
          );
          expect(
            circleRect.right,
            lessThanOrEqualTo(
              tester.view.physicalSize.width / tester.view.devicePixelRatio,
            ),
          );
          if (!own) {
            // Incoming bubbles leave room for the usual gutter.
            expect(circleRect.left, closeTo(bubbleRect.right + Grid.xxs, 0.01));
          }
          final fill =
              tester.widget<DecoratedBox>(circle).decoration as BoxDecoration;
          // The armed circle uses the chosen message color, not the app accent.
          expect(fill.color, const Color(0xFFCC1F7A));
          expect(find.text('Reply'), findsOneWidget);
          final centerBeforePulse = tester.getCenter(
            find.byKey(const ValueKey('swipe-reply-feedback')),
          );
          await tester.pump(const Duration(milliseconds: 60));
          expect(
            tester.getCenter(
              find.byKey(const ValueKey('swipe-reply-feedback')),
            ),
            centerBeforePulse,
          );
          expect(tester.getRect(circle).width, greaterThan(Grid.lg));
          await tester.pump(const Duration(milliseconds: 180));
          expect(tester.getRect(circle).width, Grid.lg);
          expect(haptics, hasLength(1));
          final feedback = find.byKey(const ValueKey('swipe-reply-feedback'));
          expect(tester.getCenter(feedback).dy, bubbleRect.center.dy);
          expect(
            tester.getCenter(find.text('Reply')).dx,
            tester.getCenter(circle).dx,
          );
          expect(
            tester.widget<Text>(find.text('Reply')).style?.color,
            tester.element(circle).colors.onSurfaceVariant,
          );
          // Retreat below readiness without returning to rest, then cross again.
          await gesture.moveBy(const Offset(210, 0));
          await tester.pump();
          expect(tester.getTopLeft(row).dx, lessThan(initial.dx));
          await gesture.moveBy(const Offset(-45, 0));
          await tester.pump();
          expect(haptics, hasLength(2));
          await tester.pump(const Duration(milliseconds: 60));
          expect(tester.getRect(circle).width, greaterThan(Grid.lg));
          await tester.pump(const Duration(milliseconds: 180));
          expect(tester.getRect(circle).width, Grid.lg);
          await gesture.up();
          await tester.pumpAndSettle();
          expect(find.byType(ThreadDetailPage), findsOneWidget);
          expect(
            tester
                .widget<ThreadDetailPage>(find.byType(ThreadDetailPage))
                .threadHead
                .id,
            threadIdRoot,
          );
          final navigator = Navigator.of(
            tester.element(find.byType(ThreadDetailPage)),
          );
          if (!thread) {
            expect(navigator.canPop(), isTrue);
            return;
          }
          // A thread head's reply destination is the page already showing:
          // its composer takes the reply and no copy of the thread stacks.
          expect(navigator.canPop(), isFalse);
          expect(
            tester
                .widget<TextField>(find.byType(TextField))
                .focusNode
                ?.hasFocus,
            isTrue,
          );
          FocusManager.instance.primaryFocus?.unfocus();
          await tester.pumpAndSettle();
          await tester.longPress(
            find
                .descendant(
                  of: row,
                  matching: find.byType(LocalSendBubbleSurface),
                )
                .first,
          );
          await tester.pumpAndSettle();
          await tester.tap(find.text('Reply'));
          await tester.pumpAndSettle();
          expect(navigator.canPop(), isFalse);
          expect(
            tester
                .widget<TextField>(find.byType(TextField))
                .focusNode
                ?.hasFocus,
            isTrue,
          );
        },
      );
    }
  }

  testWidgets('a notification for the thread in front keeps one thread page', (
    tester,
  ) async {
    PendingDeepLinkNotifier.debugUriStreamOverride = const Stream.empty();
    addTearDown(() {
      PendingDeepLinkNotifier.debugUriStreamOverride = null;
      pendingPushNotificationLink.value = null;
    });
    final events = [
      _textMsg(id: threadIdRoot, pubkey: 'alice', content: 'Thread head'),
    ];
    // The linked reply is the oldest of many, far above the thread's tail.
    final replies = [
      for (var i = 0; i < 30; i++)
        _textMsg(
          id: i == 0 ? threadIdA : 'later-reply-$i',
          pubkey: 'alice',
          content: i == 0 ? 'Linked reply' : 'Later reply $i',
          createdAt: 1001 + i,
          extraTags: [
            ['e', threadIdRoot, '', 'reply'],
          ],
        ),
    ];
    final messages = formatTimeline(events);
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        messages: events,
        threadReplies: {threadIdRoot: replies},
        home: DeepLinkDispatcher(
          child: ThreadDetailPage(
            threadHead: messages.first,
            allMessages: messages,
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final navigator = Navigator.of(
      tester.element(find.byType(ThreadDetailPage)),
    );
    final linkedRow = find.byKey(
      const ValueKey('thread-message-row-$threadIdA'),
    );
    expect(linkedRow.hitTestable(), findsNothing);
    const link = MessageDeepLink(
      channelId: _channelId,
      messageId: threadIdA,
      threadRootId: threadIdRoot,
    );

    // Tapping the reply's notification while its thread is in front lands on
    // that reply in the same page, the way opening the thread from it does.
    pendingPushNotificationLink.value = link;
    for (var frame = 0; frame < 6; frame++) {
      await tester.pump();
    }
    await tester.pump(const Duration(milliseconds: 50));
    await tester.pump(const Duration(milliseconds: 400));
    expect(navigator.canPop(), isFalse);
    expect(find.byType(ChannelDetailPage), findsNothing);
    expect(linkedRow.hitTestable(), findsOneWidget);
    final highlight =
        tester
                .widget<DecoratedBox>(
                  find.byKey(const ValueKey('thread-message-$threadIdA')),
                )
                .decoration
            as BoxDecoration;
    expect(highlight.color!.a, greaterThan(0));
    await tester.pumpAndSettle();

    // A reply below the head still opens its own branch on top.
    final swipe = await tester.startGesture(
      tester.getCenter(
        find
            .descendant(
              of: linkedRow,
              matching: find.byType(LocalSendBubbleSurface),
            )
            .first,
      ),
    );
    await swipe.moveBy(const Offset(-25, 0));
    await swipe.moveBy(const Offset(-100, 0));
    await swipe.up();
    await tester.pumpAndSettle();
    expect(navigator.canPop(), isTrue);
    expect(
      tester
          .widget<ThreadDetailPage>(find.byType(ThreadDetailPage))
          .threadHead
          .id,
      threadIdA,
    );

    // Covered by that branch, the thread is no longer in front, so the
    // notification opens it again on top.
    pendingPushNotificationLink.value = null;
    pendingPushNotificationLink.value = link;
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<ThreadDetailPage>(find.byType(ThreadDetailPage))
          .threadHead
          .id,
      threadIdRoot,
    );
  });

  for (final movement in [
    const Offset(-45, 0),
    const Offset(100, 0),
    const Offset(0, -100),
  ]) {
    testWidgets('non-reply drag $movement does not navigate', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'swipe-cancel',
              pubkey: 'alice',
              content: 'Keep reading',
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('message-row-swipe-cancel'));
      await tester.drag(
        find
            .descendant(of: row, matching: find.byType(LocalSendBubbleSurface))
            .first,
        movement,
      );
      await tester.pumpAndSettle();
      expect(find.byType(ThreadDetailPage), findsNothing);
    });
  }

  for (final reducedMotion in [false, true]) {
    testWidgets('cancelled drag resets feedback, reduced=$reducedMotion', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          disableAnimations: reducedMotion,
          messages: [
            _textMsg(id: 'cancel', pubkey: 'alice', content: 'Cancel drag'),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('message-row-cancel'));
      final origin = tester.getTopLeft(row);
      final gesture = await tester.startGesture(
        tester.getCenter(
          find
              .descendant(
                of: row,
                matching: find.byType(LocalSendBubbleSurface),
              )
              .first,
        ),
      );
      await gesture.moveBy(const Offset(-25, 0));
      await gesture.moveBy(const Offset(-90, 0));
      await tester.pump();
      final icon = find.byIcon(BuzzIcons.messageSquareReply);
      expect(
        tester
            .widget<Opacity>(
              find.ancestor(of: icon, matching: find.byType(Opacity)).first,
            )
            .opacity,
        1,
      );
      await gesture.cancel();
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(row), origin);
      expect(find.byType(ThreadDetailPage), findsNothing);
    });
  }

  testWidgets('crossing threshold then retreating cancels reply', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _textMsg(id: 'retreat', pubkey: 'alice', content: 'Keep reading'),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final row = find.byKey(const ValueKey('message-row-retreat'));
    final gesture = await tester.startGesture(
      tester.getCenter(
        find
            .descendant(of: row, matching: find.byType(LocalSendBubbleSurface))
            .first,
      ),
    );
    await gesture.moveBy(const Offset(-25, 0));
    await gesture.moveBy(const Offset(-90, 0));
    await gesture.moveBy(const Offset(70, 0));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(find.byType(ThreadDetailPage), findsNothing);
  });
}

void swipeReplyLinkTests() {
  for (final onLink in [true, false]) {
    testWidgets(
      'left drag starting ${onLink ? 'on a link' : 'beside a link'} replies',
      variant: const TargetPlatformVariant({
        TargetPlatform.iOS,
        TargetPlatform.android,
      }),
      (tester) async {
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _textMsg(
                id: 'swipe-link',
                pubkey: 'alice',
                content: 'Read [the doc](https://example.com/doc) carefully',
              ),
            ],
          ),
        );
        await tester.pumpAndSettle();
        final gesture = await tester.startGesture(
          tester.getCenter(
            onLink ? find.text('the doc') : find.textContaining('carefully'),
          ),
        );
        await gesture.moveBy(const Offset(-25, 0));
        await gesture.moveBy(const Offset(-100, 0));
        await tester.pump();
        await gesture.up();
        await tester.pumpAndSettle();
        expect(find.byType(ThreadDetailPage), findsOneWidget);
        expect(find.text('Copy link'), findsNothing);
      },
    );
  }
}
